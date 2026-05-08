# Confidence audit

I revised the package after the user noted that Codex should not receive excessive injected context.

Changes:

1. Skill reduced to a short routing rule.
2. Plugin manifest descriptions shortened.
3. MCP initialize instructions shortened.
4. Six advertised MCP tools collapsed into one `cxs(op,args)` tool.
5. MCP tool outputs are compact JSON, not pretty JSON.
6. README/docs remain available on disk but are not meant to be injected automatically.

Validated:

- bounded search over long lines;
- missing paths do not fall back to root;
- explicit files in excluded run dirs are allowed;
- default scopes exclude vendor/run dirs;
- JSONL large text fields are not searched by default;
- MCP initialize + tools/list work;
- optional `rg` shim works on common `rg -n` usage.

I do not claim mathematical certainty over every future Codex loader variant or every possible `rg` flag. The implementation is bounded, test-covered, and has fallback paths.
