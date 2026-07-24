# Helper predicates + imports (frozen, language v3)

Pins the semantics of the two language-v3 authoring constructs (R4-01
Phase C):

- **`func` helper predicates** — call-by-value boolean helpers, inlined on
  the compiled path and interpreted on the AST path; both paths must keep
  producing the pinned decisions forever.
- **`import "path" as ns`** — load-time resolution against this directory;
  the imported library's functions (including its internal `senior` →
  local-call rewrite) are embedded before evaluation.

The `suspended_never` case additionally freezes that deny-overrides
composes through a function call.

Frozen per docs/reference/DSL_COMPATIBILITY.md: decisions here may only
change behind a language-version bump with a dated waiver.
