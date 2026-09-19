<!-- grafel:mcp-usage:start v=2 -->

## grafel MCP

This repo is part of grafel group **corum-rust**. grafel is an architecture knowledge graph available via MCP. When you (an AI coding agent) need to understand how this codebase fits together, prefer the grafel MCP tools over `grep` + reading files.

### STANDING DIRECTIVE — query the graph, don't grep your way around it

- **Default to grafel for STRUCTURAL questions**: where is `X` defined, who calls/uses `Y`, how does a request flow end-to-end, what is the blast radius of a change, what are the modules. Reach for `grafel_find` / `grafel_inspect` / `grafel_related` / `grafel_trace` / `grafel_impact_radius` for these — **not** `grep` + reading files.
- **This holds for the WHOLE session, not just your first few calls.** If you notice you have been grepping or opening files to answer a structural question, stop and query the graph instead — it is faster and more accurate, and it stays that way on call 50 as much as on call 1.
- **`grep` is still right for**: raw string / substring / TODO / FIXME sweeps, and content that is not in the graph (comments, config values, log strings).

### When to use grafel instead of grep

| Question shape | Prefer |
|---|---|
| "Where is `X` defined?" | `grafel_find` |
| "What does `X` look like + detail?" | `grafel_inspect` |
| "Who calls `X` / what does `X` call?" | `grafel_related` (direction=callers\|callees) |
| "Compare reference vs candidate response/payload" | `grafel_diff` (aspect=response_shape\|payload) |
| "Trace data/control flow through X" | `grafel_trace` (kind=data\|control) |
| "What breaks if I change `X`?" | `grafel_impact_radius` |
| "Dead code / cycles / tech debt?" | `grafel_debt` (kind=dead_code\|cycles) |
| "How does the frontend talk to the backend?" | `grafel_cross_links` |
| "Show me the source of `X`" | `grafel_get_source` |

### When grep IS still better

- Substring search across all files for non-entity strings (comments, TODOs).
- Anything where you need raw file contents in bulk.

### Anti-patterns

- Don't read an entire file to find one function — `grafel_inspect` returns it directly.
- Don't glob for a class name across the repo — `grafel_find` indexes it.
- Don't traverse imports manually — `grafel_subgraph` does it via the IMPORTS edge.

The full agent guide is delivered automatically in the MCP `instructions` handshake when you connect.

_Do not edit between the markers — this block is auto-updated by `grafel install`._

<!-- grafel:mcp-usage:end -->