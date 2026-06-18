# Migration Guide — mcp-filesystem 1.x → 2.0.0

**Release theme:** MCP specification compliance (protocol revision **`2025-11-25`**).

Version 2.0.0 is a **major** release because the shape of every `tools/call`
response changed. Standard MCP clients (Claude Desktop, the MCP SDKs) already
expect the new format and need no changes. If you wrote a **custom client** that
reads the raw JSON-RPC `result` of `tools/call`, you must update it. See
[Breaking changes](#breaking-changes).

---

## Why this release

Earlier 1.x releases predated large parts of the MCP specification:

1. **`tools/call` did not return a `CallToolResult`.** Handlers returned ad-hoc
   payloads such as `{"content": "file text"}` (note: `content` was a *string*,
   not the required array) or `{"success": true}` directly as the JSON-RPC
   `result`.
2. **`initialize` advertised `resources` and `prompts` capabilities** that had
   no handlers, so a client acting on them received `-32601 Method not found`.
3. **Protocol version was pinned to `2024-11-05`** with no negotiation.

---

## Breaking changes

### 1. `tools/call` now returns a spec-compliant `CallToolResult`

**Before (1.x)** — `read_text_file`:

```jsonc
{ "content": "Hello, World!", "totalLines": 1 }
```

**After (2.0.0):**

```jsonc
{
  "content": [
    { "type": "text", "text": "{\"content\":\"Hello, World!\",\"totalLines\":1}" }
  ],
  "structuredContent": { "content": "Hello, World!", "totalLines": 1 },
  "isError": false
}
```

- The original payload is preserved under **`structuredContent`** and as
  serialized text in **`content[0].text`**.
- **Migration:** read `result.structuredContent` (or `result.content[0].text`)
  instead of `result` directly.

### 2. `read_media_file` returns typed `ImageContent` / `AudioContent`

For recognised media types, `read_media_file` now returns spec content items
instead of a base64 string under a `content` key:

```jsonc
{
  "content": [{ "type": "image", "data": "<base64>", "mimeType": "image/png" }],
  "isError": false
}
```

Non-media binaries return a text note with the base64 under
`structuredContent.data`. **Migration:** read image/audio bytes from
`result.content[0].data`; read other binaries from `structuredContent.data`.

### 3. Tool failures are returned as results, not protocol errors

Execution failures (path-not-found, sandbox violations, read-only policy
rejections, etc.) are now `CallToolResult`s with `isError: true` rather than
JSON-RPC errors, so the model can read the message and try another path.
**Protocol-level errors** (malformed request, missing `name`, unknown
tool/method) remain JSON-RPC `error` objects.

**Migration:** check `result.isError` on every `tools/call` response.

### 4. `resources` and `prompts` capabilities are no longer advertised

They were never implemented. **Migration:** none — clients now correctly see
they are unavailable. Resources (`file://` URIs) are on the roadmap.

### 5. Negotiated protocol version is now `2025-11-25`

`initialize` performs version negotiation: a supported requested revision
(`2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`) is echoed back;
otherwise the latest is offered. Clients pinned to `2024-11-05` keep working.

---

## New in 2.0.0

- **`instructions`** field in `InitializeResult`.
- **`structuredContent`** on object-valued tool results (MCP 2025-06-18+).
- **Version negotiation** in `initialize`.
- **Typed media content** (`ImageContent` / `AudioContent`) from `read_media_file`.

---

## Not yet implemented (roadmap)

Intentionally **not** advertised as capabilities until implemented:

| Feature | Notes |
|---|---|
| `resources/*` (`file://…` URIs) | files/dirs as readable resources — the natural fit for a filesystem server |
| `prompts/*` | `organize-directory`, `code-review` |
| Streamable HTTP transport | current HTTP transport is POST `/rpc` |
| TLS on HTTP transport | plaintext today |
| `roots/list` | request allowed roots from the client instead of CLI flags |
| `logging`, `completion`, progress, cancellation | — |

---

## Upgrade checklist

- [ ] Reinstall: `cargo install mcp-filesystem`.
- [ ] Standard MCP client: nothing to do.
- [ ] Custom client:
  - [ ] Read tool output from `result.structuredContent` (or `content[0].text`).
  - [ ] Read media from `result.content[0].data`.
  - [ ] Check `result.isError` to detect tool failures.
  - [ ] Stop relying on `resources`/`prompts` capabilities.
