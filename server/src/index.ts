// The sync service: accounts, each account's library rows in D1, and its book text and covers in
// KV. Books never change once stored, so KV's delayed consistency doesn't matter for them.

interface Env {
  DB: D1Database;
  BLOBS: KVNamespace;
}

interface User {
  id: string;
  email: string;
}

const PBKDF2_ROUNDS = 60_000;
const MAX_FAILURES = 10;
const FAILURE_WINDOW_MS = 15 * 60 * 1000;
const MAX_BLOB_BYTES = 25 * 1024 * 1024;
const BOOK_ID = /^[0-9a-f]{64}$/;
const COVER_NAME = /^[0-9a-f]{64}\.(jpg|png|gif|webp)$/;

class Refused extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

const hex = (bytes: ArrayBuffer | Uint8Array) =>
  [...new Uint8Array(bytes)].map((b) => b.toString(16).padStart(2, "0")).join("");

const randomHex = (length: number) => hex(crypto.getRandomValues(new Uint8Array(length)));

async function sha256(text: string): Promise<string> {
  return hex(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));
}

async function hashPassword(password: string, salt: string): Promise<string> {
  const key = await crypto.subtle.importKey("raw", new TextEncoder().encode(password), "PBKDF2", false, ["deriveBits"]);
  const bits = await crypto.subtle.deriveBits(
    { name: "PBKDF2", hash: "SHA-256", salt: new TextEncoder().encode(salt), iterations: PBKDF2_ROUNDS },
    key,
    256,
  );
  return hex(bits);
}

function sameText(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let difference = 0;
  for (let i = 0; i < a.length; i++) difference |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return difference === 0;
}

function parseCredentials(body: unknown): { email: string; password: string } {
  const { email, password } = (body ?? {}) as Record<string, unknown>;
  if (typeof email !== "string" || !/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email) || email.length > 254) {
    throw new Refused(400, "That email address doesn’t look right.");
  }
  if (typeof password !== "string" || password.length < 8 || password.length > 256) {
    throw new Refused(400, "Passwords need at least 8 characters.");
  }
  return { email: email.trim().toLowerCase(), password };
}

async function startSession(env: Env, user: User): Promise<Response> {
  const token = randomHex(32);
  await env.DB.prepare("INSERT INTO sessions (token_hash, user_id, created_at) VALUES (?, ?, ?)")
    .bind(await sha256(token), user.id, Date.now())
    .run();
  return json({ token, email: user.email });
}

async function signUp(request: Request, env: Env): Promise<Response> {
  const { email, password } = parseCredentials(await request.json());
  const taken = await env.DB.prepare("SELECT id FROM users WHERE email = ?").bind(email).first();
  if (taken) throw new Refused(409, "That email already has an account. Sign in instead.");
  const salt = randomHex(16);
  const user = { id: crypto.randomUUID(), email };
  await env.DB.prepare("INSERT INTO users (id, email, password_hash, password_salt, created_at) VALUES (?, ?, ?, ?, ?)")
    .bind(user.id, email, await hashPassword(password, salt), salt, Date.now())
    .run();
  return startSession(env, user);
}

async function signIn(request: Request, env: Env): Promise<Response> {
  const { email, password } = parseCredentials(await request.json());
  const now = Date.now();
  const failures = await env.DB.prepare("SELECT count, window_start FROM sign_in_failures WHERE email = ?")
    .bind(email)
    .first<{ count: number; window_start: number }>();
  const inWindow = failures !== null && now - failures.window_start < FAILURE_WINDOW_MS;
  if (inWindow && failures.count >= MAX_FAILURES) {
    throw new Refused(429, "Too many tries. Wait 15 minutes and try again.");
  }
  const row = await env.DB.prepare("SELECT id, email, password_hash, password_salt FROM users WHERE email = ?")
    .bind(email)
    .first<User & { password_hash: string; password_salt: string }>();
  const hash = await hashPassword(password, row?.password_salt ?? "no-such-account");
  if (!row || !sameText(hash, row.password_hash)) {
    await env.DB.prepare(
      "INSERT INTO sign_in_failures (email, count, window_start) VALUES (?, 1, ?) ON CONFLICT(email) DO UPDATE SET count = ?, window_start = ?",
    )
      .bind(email, now, inWindow ? failures.count + 1 : 1, inWindow ? failures.window_start : now)
      .run();
    throw new Refused(401, "That email and password don’t match.");
  }
  await env.DB.prepare("DELETE FROM sign_in_failures WHERE email = ?").bind(email).run();
  return startSession(env, { id: row.id, email: row.email });
}

async function authenticate(request: Request, env: Env): Promise<User & { tokenHash: string }> {
  const token = request.headers.get("authorization")?.replace(/^Bearer /, "");
  if (!token) throw new Refused(401, "Sign in to sync.");
  const tokenHash = await sha256(token);
  const user = await env.DB.prepare(
    "SELECT users.id, users.email FROM sessions JOIN users ON users.id = sessions.user_id WHERE token_hash = ?",
  )
    .bind(tokenHash)
    .first<User>();
  if (!user) throw new Refused(401, "Sign in to sync.");
  return { ...user, tokenHash };
}

const blobKey = (user: User, kind: string, name: string) => `${user.id}/${kind}/${name}`;

function checkBlobName(kind: string, name: string) {
  const valid = (kind === "book" && BOOK_ID.test(name)) || (kind === "cover" && COVER_NAME.test(name));
  if (!valid) throw new Refused(400, "That isn’t a book or cover name.");
}

async function getLibrary(env: Env, user: User): Promise<Response> {
  const books = await env.DB.prepare(
    "SELECT id, title, author, format, chapters, cover, added, chapter, fraction, position_at FROM books WHERE user_id = ?",
  )
    .bind(user.id)
    .all();
  const settings = await env.DB.prepare("SELECT json, updated_at FROM settings WHERE user_id = ?")
    .bind(user.id)
    .first<{ json: string; updated_at: number }>();
  return json({ books: books.results, settings: settings ? { json: settings.json, updated_at: settings.updated_at } : null });
}

async function putBook(request: Request, env: Env, user: User, id: string): Promise<Response> {
  if (!BOOK_ID.test(id)) throw new Refused(400, "That isn’t a book id.");
  const stored = await env.BLOBS.get(blobKey(user, "book", id), "stream");
  if (!stored) throw new Refused(409, "Upload the book’s text first.");
  await stored.cancel();
  const b = (await request.json()) as Record<string, unknown>;
  const isNumber = (v: unknown) => typeof v === "number" && Number.isFinite(v) && v >= 0;
  if (
    typeof b.title !== "string" ||
    (b.format !== "Epub" && b.format !== "Pdf") ||
    !isNumber(b.chapters) ||
    !isNumber(b.added) ||
    !isNumber(b.chapter) ||
    !isNumber(b.fraction) ||
    !isNumber(b.position_at)
  ) {
    throw new Refused(400, "That book row is incomplete.");
  }
  // The later position wins, whichever device reaches the server first.
  await env.DB.prepare(
    `INSERT INTO books (user_id, id, title, author, format, chapters, cover, added, chapter, fraction, position_at)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
     ON CONFLICT(user_id, id) DO UPDATE SET
       cover = COALESCE(excluded.cover, books.cover),
       chapter = CASE WHEN excluded.position_at > books.position_at THEN excluded.chapter ELSE books.chapter END,
       fraction = CASE WHEN excluded.position_at > books.position_at THEN excluded.fraction ELSE books.fraction END,
       position_at = MAX(excluded.position_at, books.position_at)`,
  )
    .bind(user.id, id, b.title, b.author ?? null, b.format, b.chapters, b.cover ?? null, b.added, b.chapter, b.fraction, b.position_at)
    .run();
  return json({ ok: true });
}

async function putSettings(request: Request, env: Env, user: User): Promise<Response> {
  const body = (await request.json()) as { json?: unknown; updated_at?: unknown };
  if (typeof body.json !== "string" || typeof body.updated_at !== "number") {
    throw new Refused(400, "Those settings are incomplete.");
  }
  await env.DB.prepare(
    `INSERT INTO settings (user_id, json, updated_at) VALUES (?, ?, ?)
     ON CONFLICT(user_id) DO UPDATE SET
       json = CASE WHEN excluded.updated_at > settings.updated_at THEN excluded.json ELSE settings.json END,
       updated_at = MAX(excluded.updated_at, settings.updated_at)`,
  )
    .bind(user.id, body.json, body.updated_at)
    .run();
  return json({ ok: true });
}

async function deleteAccount(env: Env, user: User): Promise<Response> {
  let cursor: string | undefined;
  do {
    const page = await env.BLOBS.list({ prefix: `${user.id}/`, cursor });
    await Promise.all(page.keys.map((k) => env.BLOBS.delete(k.name)));
    cursor = page.list_complete ? undefined : page.cursor;
  } while (cursor);
  await env.DB.batch([
    env.DB.prepare("DELETE FROM books WHERE user_id = ?").bind(user.id),
    env.DB.prepare("DELETE FROM settings WHERE user_id = ?").bind(user.id),
    env.DB.prepare("DELETE FROM sessions WHERE user_id = ?").bind(user.id),
    env.DB.prepare("DELETE FROM users WHERE id = ?").bind(user.id),
  ]);
  return json({ ok: true });
}

async function route(request: Request, env: Env): Promise<Response> {
  const { pathname } = new URL(request.url);
  const method = request.method;
  if (method === "POST" && pathname === "/v1/sign-up") return signUp(request, env);
  if (method === "POST" && pathname === "/v1/sign-in") return signIn(request, env);

  const user = await authenticate(request, env);
  if (method === "POST" && pathname === "/v1/sign-out") {
    await env.DB.prepare("DELETE FROM sessions WHERE token_hash = ?").bind(user.tokenHash).run();
    return json({ ok: true });
  }
  if (method === "DELETE" && pathname === "/v1/account") return deleteAccount(env, user);
  if (method === "GET" && pathname === "/v1/library") return getLibrary(env, user);
  if (method === "PUT" && pathname === "/v1/settings") return putSettings(request, env, user);

  const book = pathname.match(/^\/v1\/books\/([^/]+)$/);
  if (method === "PUT" && book) return putBook(request, env, user, book[1]);

  const blob = pathname.match(/^\/v1\/blobs\/(book|cover)\/([^/]+)$/);
  if (blob) {
    const [, kind, name] = blob;
    checkBlobName(kind, name);
    const key = blobKey(user, kind, name);
    if (method === "GET") {
      const stored = await env.BLOBS.get(key, "stream");
      return stored ? new Response(stored) : json({ error: "Not found." }, 404);
    }
    if (method === "PUT") {
      const bytes = await request.arrayBuffer();
      if (bytes.byteLength > MAX_BLOB_BYTES) throw new Refused(413, "That book is too large to sync.");
      await env.BLOBS.put(key, bytes);
      return json({ ok: true });
    }
  }
  return json({ error: "Not found." }, 404);
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    try {
      return await route(request, env);
    } catch (e) {
      if (e instanceof Refused) return json({ error: e.message }, e.status);
      if (e instanceof SyntaxError) return json({ error: "That request wasn’t valid." }, 400);
      console.error(e);
      return json({ error: "Something went wrong on the server." }, 500);
    }
  },
} satisfies ExportedHandler<Env>;
