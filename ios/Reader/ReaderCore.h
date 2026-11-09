// The C ABI `crates/reader-ios` exports to the Swift shell. One ReaderHost per Rune view.
// Every call runs on the main thread. Points are UIKit points; `scale` is the screen scale.
#pragma once
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct ReaderHost ReaderHost;

// What the host should do after a frame. Mirrors `reader_app::HostRequest`.
typedef enum {
    READER_REQUEST_NONE = 0,
    READER_REQUEST_PICK_BOOK = 1,
    READER_REQUEST_FIND_BOOKS = 2,
    READER_REQUEST_SIGN_IN = 3,
    READER_REQUEST_CREATE_ACCOUNT = 4,
    READER_REQUEST_ADD_FOLDER = 5,
    READER_REQUEST_FORGET_FOLDERS = 6,
} ReaderRequest;

typedef enum {
    READER_TOUCH_BEGAN = 0,
    READER_TOUCH_MOVED = 1,
    READER_TOUCH_ENDED = 2,
    READER_TOUCH_CANCELLED = 3,
} ReaderTouchPhase;

// Called on the main thread when the view wants frames again after the host paused its
// display link: background work (a parse) finished, or a timer came due. Never called after
// `reader_host_free`.
typedef void (*ReaderWakeFn)(void *context);

// `layer` is a CAMetalLayer*. `data_dir` is a UTF-8 path the library is kept in.
// `wake` may be NULL. Returns NULL on failure.
ReaderHost *reader_host_new(void *layer, float width, float height, float scale, bool dark, const char *data_dir,
                            ReaderWakeFn wake, void *wake_context);
void reader_host_free(ReaderHost *host);

void reader_host_resize(ReaderHost *host, float width, float height, float scale);
void reader_host_set_insets(ReaderHost *host, float top, float right, float bottom, float left);
void reader_host_set_dark(ReaderHost *host, bool dark);

// Steps and presents one frame. Returns true while the view wants more frames
// (animating, flinging, or input pending); the host pauses its display link when false.
bool reader_host_frame(ReaderHost *host);

// `time` is seconds on a steady clock (CACurrentMediaTime). `id` tells fingers apart.
void reader_host_touch(ReaderHost *host, uint64_t id, ReaderTouchPhase phase, float x, float y, double time);

// Drains one pending request; call after each frame until it returns NONE.
ReaderRequest reader_host_take_request(ReaderHost *host);

// A file the document picker returned, already copied into the app's sandbox.
void reader_host_import(ReaderHost *host, const char *path);

// Signs in with, or creates an account from, what the host asked for.
void reader_host_submit_credentials(ReaderHost *host, const char *email, const char *password, bool create);

// Searches folders for books. The host keeps them readable until the next search.
void reader_host_search_folders(ReaderHost *host, const char *const *paths, size_t count);

// Whether the page the last frame drew is dark, so the status bar can match it.
bool reader_host_page_dark(ReaderHost *host);
