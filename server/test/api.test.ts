// Runs against a real local Worker with its own D1 and KV: `bun run dev`, then `bun test test/`.
import { describe, expect, test } from "bun:test";

const BASE = process.env.READER_API ?? "http://127.0.0.1:8799";
const bookId = (seed: string) => seed.repeat(64).slice(0, 64);
const unique = () => `t${crypto.randomUUID().slice(0, 12)}@example.com`;

const call = (path: string, init: RequestInit & { token?: string } = {}) =>
  fetch(BASE + path, {
    ...init,
    headers: { ...(init.token ? { authorization: `Bearer ${init.token}` } : {}), ...init.headers },
  });

async function account(email = unique(), password = "correct horse") {
  const res = await call("/v1/sign-up", { method: "POST", body: JSON.stringify({ email, password }) });
  expect(res.status).toBe(200);
  const { token } = (await res.json()) as { token: string };
  return { email, password, token };
}

const row = (over: Record<string, unknown> = {}) => ({
  title: "A Book",
  author: "Someone",
  format: "Epub",
  chapters: 3,
  cover: null,
  added: 1,
  chapter: 0,
  fraction: 0,
  position_at: 0,
  ...over,
});

const library = async (token: string) => (await (await call("/v1/library", { token })).json()) as any;
const putBlob = (token: string, id: string, body = "{}") => call(`/v1/blobs/book/${id}`, { method: "PUT", token, body });
const putRow = (token: string, id: string, over = {}) =>
  call(`/v1/books/${id}`, { method: "PUT", token, body: JSON.stringify(row(over)) });

describe("accounts", () => {
  test("sign up, sign in and sign out", async () => {
    const me = await account();
    const res = await call("/v1/sign-in", { method: "POST", body: JSON.stringify({ email: me.email.toUpperCase(), password: me.password }) });
    expect(res.status).toBe(200);
    const { token } = (await res.json()) as { token: string };
    expect((await call("/v1/library", { token })).status).toBe(200);
    expect((await call("/v1/sign-out", { method: "POST", token })).status).toBe(200);
    expect((await call("/v1/library", { token })).status).toBe(401);
    expect((await call("/v1/library", { token: me.token })).status).toBe(200);
  });

  test("refuses a taken email, a wrong password and a short one", async () => {
    const me = await account();
    expect((await call("/v1/sign-up", { method: "POST", body: JSON.stringify({ email: me.email, password: "another password" }) })).status).toBe(409);
    expect((await call("/v1/sign-in", { method: "POST", body: JSON.stringify({ email: me.email, password: "wrong password" }) })).status).toBe(401);
    expect((await call("/v1/sign-up", { method: "POST", body: JSON.stringify({ email: unique(), password: "short" }) })).status).toBe(400);
  });

  test("locks sign-in after repeated failures, even with the right password", async () => {
    const me = await account();
    const attempt = (password: string) => call("/v1/sign-in", { method: "POST", body: JSON.stringify({ email: me.email, password }) });
    for (let i = 0; i < 10; i++) expect((await attempt("wrong password")).status).toBe(401);
    expect((await attempt(me.password)).status).toBe(429);
  });

  test("nothing works without a valid token", async () => {
    expect((await call("/v1/library")).status).toBe(401);
    expect((await call("/v1/library", { token: "nope" })).status).toBe(401);
    expect((await call(`/v1/blobs/book/${bookId("a")}`, { token: "nope" })).status).toBe(401);
  });

  test("deleting an account removes its data and sessions", async () => {
    const me = await account();
    const id = bookId("d");
    await putBlob(me.token, id, "text");
    await putRow(me.token, id);
    expect((await call("/v1/account", { method: "DELETE", token: me.token })).status).toBe(200);
    expect((await call("/v1/library", { token: me.token })).status).toBe(401);
    expect((await call("/v1/sign-in", { method: "POST", body: JSON.stringify({ email: me.email, password: me.password }) })).status).toBe(401);
    const again = await account(me.email);
    expect((await library(again.token)).books).toEqual([]);
    expect((await call(`/v1/blobs/book/${id}`, { token: again.token })).status).toBe(404);
  });
});

describe("library", () => {
  test("one account can’t see another’s books or files", async () => {
    const a = await account();
    const b = await account();
    const id = bookId("b");
    await putBlob(a.token, id, "secret text");
    await putRow(a.token, id);
    expect((await library(b.token)).books).toEqual([]);
    expect((await call(`/v1/blobs/book/${id}`, { token: b.token })).status).toBe(404);
    expect(await (await call(`/v1/blobs/book/${id}`, { token: a.token })).text()).toBe("secret text");
  });

  test("a row can’t be written before its book’s text", async () => {
    const me = await account();
    expect((await putRow(me.token, bookId("c"))).status).toBe(409);
    expect((await library(me.token)).books).toEqual([]);
  });

  test("the later reading position wins, in either arrival order", async () => {
    const me = await account();
    const id = bookId("e");
    await putBlob(me.token, id);
    await putRow(me.token, id, { chapter: 5, fraction: 0.5, position_at: 200 });
    await putRow(me.token, id, { chapter: 1, fraction: 0.1, position_at: 100 });
    let [book] = (await library(me.token)).books;
    expect([book.chapter, book.fraction, book.position_at]).toEqual([5, 0.5, 200]);
    await putRow(me.token, id, { chapter: 9, fraction: 0.9, position_at: 300 });
    [book] = (await library(me.token)).books;
    expect([book.chapter, book.fraction, book.position_at]).toEqual([9, 0.9, 300]);
  });

  test("the later settings win", async () => {
    const me = await account();
    const put = (json: string, updated_at: number) => call("/v1/settings", { method: "PUT", token: me.token, body: JSON.stringify({ json, updated_at }) });
    await put('{"theme":"Night"}', 200);
    await put('{"theme":"Paper"}', 100);
    expect((await library(me.token)).settings).toEqual({ json: '{"theme":"Night"}', updated_at: 200 });
  });

  test("refuses names that could reach outside an account’s files", async () => {
    const me = await account();
    for (const path of ["/v1/blobs/book/..%2F..%2Fother", "/v1/blobs/cover/nothex.jpg", `/v1/blobs/cover/${bookId("a")}.exe`]) {
      expect((await call(path, { method: "PUT", token: me.token, body: "x" })).status).toBe(400);
    }
  });
});
