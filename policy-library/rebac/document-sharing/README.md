# Drive-style document sharing (Zanzibar model)

Owner/editor/viewer edges, nested groups (`user → team → org`), and
folder-inheritance (`doc → folder → root`) via three builtins:
`rebac::related`, `rebac::reachable(..., "member_of", 3)`,
`rebac::inherited(..., "parent", 4)`. All bounded and cycle-safe; the direct
check compiles to a ~18ns graph lookup.

Try: `reaper-cli library run rebac/document-sharing`
