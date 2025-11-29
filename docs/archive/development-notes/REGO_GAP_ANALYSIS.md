# Rego vs Reaper DSL: Comprehensive Gap Analysis

**Date**: 2025-11-27
**Purpose**: Identify all functionality gaps between OPA/Rego and Reaper's current DSL
**Status**: Analysis Complete - Ready for Prioritization

---

## Executive Summary

**Current State**: Reaper DSL is a **simplified subset** of Rego focused on high-performance RBAC.

**Gap Count**: **~150+ features** missing from Rego
**Severity**: **MAJOR** - Rego is significantly more feature-rich

**Recommendation**: Implement Rego compatibility layer or extend Reaper DSL selectively based on user needs.

---

## 1. Data Types & Structures

### ✅ Reaper HAS (5/7)
- Strings
- Integers (i64)
- Floats (f64)
- Booleans
- Null

### ❌ Reaper MISSING (2/7)
- **Arrays** (ordered collections)
  - Rego: `[1, 2, 3]`, `["foo", "bar"]`
  - Reaper: None
  - **Impact**: CRITICAL - Can't represent lists of resources, permissions

- **Objects** (key-value maps)
  - Rego: `{"name": "alice", "role": "admin"}`
  - Reaper: None
  - **Impact**: CRITICAL - Can't represent complex data structures

- **Sets** (unordered unique collections)
  - Rego: `{"foo", "bar"}`
  - Reaper: None
  - **Impact**: HIGH - Can't represent permission sets efficiently

---

## 2. Operators

### ✅ Reaper HAS (6/12)
- `==` (Equal)
- `!=` (NotEqual)
- `>` (GreaterThan)
- `<` (LessThan)
- `>=` (GreaterEqual)
- `<=` (LessEqual)

### ❌ Reaper MISSING (6/12)
- **Arithmetic Operators**:
  - `+` (addition)
  - `-` (subtraction)
  - `*` (multiplication)
  - `/` (division)
  - `%` (modulo)
  - **Impact**: MEDIUM - Can't do math in policies (age calculations, quotas)

- **Set Operators**:
  - `&` (intersection)
  - `|` (union)
  - `-` (difference)
  - **Impact**: HIGH - Can't combine permission sets

- **Membership Operator**:
  - `in` (check if element in collection)
  - **Impact**: CRITICAL - Can't check if user in group, resource in list

---

## 3. Logical Constructs

### ✅ Reaper HAS (3/6)
- `AND` (logical and)
- `OR` (logical or)
- `NOT` (logical not)

### ❌ Reaper MISSING (3/6)
- **Universal Quantification**:
  - `every` keyword
  - Rego: `every item in items { item.status == "active" }`
  - **Impact**: HIGH - Can't express "all resources must..."

- **Existential Quantification (implicit)**:
  - `some` keyword
  - Rego: `some x in users; x.role == "admin"`
  - **Impact**: MEDIUM - Can't explicitly iterate with variable binding

- **Else Chaining**:
  - `else` keyword for fallback rules
  - Rego: `allow { ... } else { default }`
  - **Impact**: MEDIUM - Can't express cascading policies

---

## 4. Variable Binding & Assignment

### ✅ Reaper HAS (1/4)
- Entity attribute access: `user.role`, `resource.type`

### ❌ Reaper MISSING (3/4)
- **Local Variable Assignment**:
  - `:=` operator
  - Rego: `username := input.user.name`
  - **Impact**: CRITICAL - Can't store intermediate values

- **Unification**:
  - `=` operator (assignment + comparison)
  - Rego: `x = input.user; x.role == "admin"`
  - **Impact**: MEDIUM - Can't bind and compare in one step

- **Destructuring**:
  - Array/object unpacking
  - Rego: `[first, second] := input.data`
  - **Impact**: MEDIUM - Can't extract multiple values at once

---

## 5. Comprehensions

### ✅ Reaper HAS (0/3)
- None

### ❌ Reaper MISSING (3/3)
- **Set Comprehensions**:
  - Rego: `admins := {user | users[user]; users[user].role == "admin"}`
  - **Impact**: CRITICAL - Core Rego idiom for building collections

- **Array Comprehensions**:
  - Rego: `names := [user.name | users[user]]`
  - **Impact**: HIGH - Can't transform collections

- **Object Comprehensions**:
  - Rego: `{user: role | users[user]; role := users[user].role}`
  - **Impact**: HIGH - Can't build dynamic maps

---

## 6. Functions

### ✅ Reaper HAS (0/2)
- None

### ❌ Reaper MISSING (2/2)
- **User-Defined Functions**:
  - Rego: `f(x, y) := x + y if { x > 0; y > 0 }`
  - **Impact**: CRITICAL - Can't reuse logic, compose policies

- **Incremental Functions**:
  - Multiple definitions with pattern matching
  - Rego: `f(0) := "zero"` + `f(x) := "positive" if x > 0`
  - **Impact**: MEDIUM - Can't define piecewise functions

---

## 7. Built-in Functions

### ✅ Reaper HAS (0/200+)
- None (no built-in functions at all!)

### ❌ Reaper MISSING (~200+ functions)

#### Aggregates (6 functions)
- `count()`, `max()`, `min()`, `product()`, `sort()`, `sum()`
- **Impact**: CRITICAL - Can't count resources, find max values

#### Arrays (3 functions)
- `array.concat()`, `array.reverse()`, `array.slice()`
- **Impact**: HIGH - Can't manipulate lists

#### Strings (30+ functions)
- `concat()`, `contains()`, `endswith()`, `startswith()`, `split()`, `lower()`, `upper()`, `trim()`, `replace()`, `sprintf()`, `substring()`, etc.
- **Impact**: CRITICAL - Can't process strings (email validation, path matching)

#### Sets (2 functions)
- `intersection()`, `union()`
- **Impact**: HIGH - Can't combine permission sets

#### Objects (11 functions)
- `object.get()`, `object.keys()`, `object.filter()`, `object.remove()`, `object.union()`, etc.
- **Impact**: CRITICAL - Can't work with complex data

#### Type Checking (7 functions)
- `is_array()`, `is_boolean()`, `is_null()`, `is_number()`, `is_object()`, `is_set()`, `is_string()`
- **Impact**: MEDIUM - Can't validate input types

#### Encoding (14 functions)
- Base64: `base64.encode()`, `base64.decode()`
- JSON: `json.marshal()`, `json.unmarshal()`, `json.is_valid()`
- YAML: `yaml.marshal()`, `yaml.unmarshal()`
- URL: `urlquery.encode()`, `urlquery.decode()`
- Hex: `hex.encode()`, `hex.decode()`
- **Impact**: HIGH - Can't parse/encode data formats

#### Cryptography (16 functions)
- Hashing: `crypto.md5()`, `crypto.sha1()`, `crypto.sha256()`, `crypto.sha512()`
- HMAC: `crypto.hmac.md5()`, `crypto.hmac.sha256()`, etc.
- X.509: `crypto.x509.parse_certificates()`, etc.
- **Impact**: MEDIUM - Can't verify signatures, validate certs

#### JWT (16 functions)
- Verify: `io.jwt.verify_rs256()`, `io.jwt.verify_hs256()`, etc.
- Decode: `io.jwt.decode()`, `io.jwt.decode_verify()`
- Sign: `io.jwt.encode_sign()`
- **Impact**: HIGH - Can't validate JWT tokens (common auth pattern)

#### Regex (7 functions)
- `regex.match()`, `regex.find_n()`, `regex.replace()`, `regex.split()`, `regex.is_valid()`, etc.
- **Impact**: HIGH - Can't do pattern matching (email, IP, etc.)

#### Networking (7 functions)
- `net.cidr_contains()`, `net.cidr_expand()`, `net.cidr_merge()`, `net.lookup_ip_addr()`, etc.
- **Impact**: MEDIUM - Can't validate IP addresses, CIDR ranges

#### Time (11 functions)
- `time.now_ns()`, `time.parse_ns()`, `time.format()`, `time.diff()`, `time.add_date()`, etc.
- **Impact**: CRITICAL - Can't do time-based policies (expiration, windows)

#### HTTP (1 function)
- `http.send()` - Make HTTP requests from policies
- **Impact**: MEDIUM - Can't fetch external data

#### GraphQL (6 functions)
- `graphql.parse()`, `graphql.is_valid()`, `graphql.parse_and_verify()`, etc.
- **Impact**: LOW - Niche use case

#### Glob (2 functions)
- `glob.match()`, `glob.quote_meta()`
- **Impact**: MEDIUM - Can't do file path matching

#### Graphs (3 functions)
- `graph.reachable()`, `graph.reachable_paths()`, `walk()`
- **Impact**: MEDIUM - Can't traverse nested structures

#### Numbers (9 functions)
- `abs()`, `ceil()`, `floor()`, `round()`, `numbers.range()`, `rand.intn()`, etc.
- **Impact**: LOW - Nice to have for quotas, sampling

#### Semantic Versioning (2 functions)
- `semver.compare()`, `semver.is_valid()`
- **Impact**: LOW - Niche use case

#### Units (2 functions)
- `units.parse()`, `units.parse_bytes()`
- **Impact**: LOW - Parse "10MB", "5h", etc.

#### UUID (2 functions)
- `uuid.parse()`, `uuid.rfc4122()`
- **Impact**: LOW - Generate/validate UUIDs

#### Bits (6 functions)
- `bits.and()`, `bits.or()`, `bits.xor()`, `bits.lsh()`, `bits.rsh()`, `bits.negate()`
- **Impact**: LOW - Bit manipulation (rare)

#### Tracing (1 function)
- `trace()` - Debug output
- **Impact**: LOW - Debugging tool

#### OPA Runtime (1 function)
- `opa.runtime()` - Get OPA version/config
- **Impact**: LOW - Introspection

#### AWS Provider (1 function)
- `providers.aws.sign_req()` - Sign AWS requests
- **Impact**: LOW - AWS-specific

---

## 8. References & Data Access

### ✅ Reaper HAS (1/5)
- Dot notation: `user.role`, `resource.type`

### ❌ Reaper MISSING (4/5)
- **Bracket Notation**:
  - Rego: `users["alice"]`, `resources[0]`
  - **Impact**: CRITICAL - Can't access dynamic keys

- **Variable Keys**:
  - Rego: `users[username]` where username is a variable
  - **Impact**: CRITICAL - Can't do dynamic lookups

- **Nested Access**:
  - Rego: `data.users[username].roles[0].permissions`
  - **Impact**: HIGH - Can't traverse deep structures

- **Wildcard Iterator**:
  - Rego: `users[_]` - iterate over all users
  - **Impact**: HIGH - Can't iterate collections

---

## 9. Iteration & Quantification

### ✅ Reaper HAS (0/3)
- None

### ❌ Reaper MISSING (3/3)
- **`in` Operator** (membership + iteration):
  - Rego: `x in [1,2,3]`, `some x in users`
  - **Impact**: CRITICAL - Can't iterate or check membership

- **`every` Keyword** (universal quantification):
  - Rego: `every user in users { user.verified }`
  - **Impact**: HIGH - Can't express "all must..."

- **`some` Keyword** (existential quantification):
  - Rego: `some user in users; user.role == "admin"`
  - **Impact**: HIGH - Can't express "exists at least one..."

---

## 10. Rule Types & Patterns

### ✅ Reaper HAS (2/5)
- **Boolean Rules**: `allow if { user.role == "admin" }`
- **Default Decision**: `default: deny`

### ❌ Reaper MISSING (3/5)
- **Complete Definitions**:
  - Rego: `admin_users := {"alice", "bob"}`
  - **Impact**: HIGH - Can't define constants/lookups

- **Partial Sets**:
  - Rego: `admins[user] { users[user].role == "admin" }`
  - **Impact**: CRITICAL - Core Rego idiom for building sets

- **Partial Objects**:
  - Rego: `user_roles[user] := role { users[user].role == role }`
  - **Impact**: HIGH - Can't build dynamic maps

- **Incremental Rules**:
  - Multiple definitions that union
  - Rego: `allow { ... }` + `allow { ... }` = union of both
  - **Impact**: HIGH - Can't compose policies from multiple rules

- **Reference Heads**:
  - Rego: `deny["msg"] := "Unauthorized"`
  - **Impact**: MEDIUM - Can't structure complex outputs

---

## 11. Modules & Imports

### ✅ Reaper HAS (1/4)
- Policy name (similar to package)

### ❌ Reaper MISSING (3/4)
- **Package Declaration**:
  - Rego: `package authz.rbac`
  - **Impact**: MEDIUM - No namespacing

- **Import Statements**:
  - Rego: `import data.users`, `import input.user as u`
  - **Impact**: HIGH - Can't reuse policies, modularize

- **Data References**:
  - Rego: `data.roles`, `data.permissions`
  - **Impact**: CRITICAL - Can't access external data stores

- **Input References**:
  - Rego: `input.user`, `input.resource`
  - **Impact**: Currently using `user.`, `resource.` directly

---

## 12. Metadata & Annotations

### ✅ Reaper HAS (1/3)
- Policy metadata (simple key-value)

### ❌ Reaper MISSING (2/3)
- **Structured Metadata**:
  - Rego: `# METADATA` blocks with YAML
  - Fields: title, description, authors, organizations, custom
  - **Impact**: LOW - Documentation/tooling

- **Schema Annotations**:
  - Rego: JSON Schema for type checking
  - **Impact**: MEDIUM - Type safety, validation

---

## 13. Testing & Debugging

### ✅ Reaper HAS (0/4)
- None

### ❌ Reaper MISSING (4/4)
- **Test Rules**:
  - Rego: `test_admin_allowed { ... }`
  - **Impact**: CRITICAL - No built-in testing framework

- **`with` Keyword** (mocking):
  - Rego: `allow with input as {"user": "alice"}`
  - **Impact**: CRITICAL - Can't mock data for tests

- **Test Naming Convention**:
  - `test_` prefix, `_test` package suffix
  - **Impact**: MEDIUM - Testing standards

- **`trace()` Function**:
  - Debug output during evaluation
  - **Impact**: MEDIUM - Debugging policies

---

## 14. Advanced Features

### ✅ Reaper HAS (1/8)
- First-match-wins semantics (SimplePolicyEvaluator)

### ❌ Reaper MISSING (7/8)
- **Partial Evaluation**:
  - Compile policies for specific contexts
  - **Impact**: MEDIUM - Performance optimization

- **WASM Compilation**:
  - Compile Rego to WebAssembly
  - **Impact**: LOW - Edge deployment (though we're fast enough)

- **Bundles**:
  - Policy + data packaging/distribution
  - **Impact**: MEDIUM - Deployment, versioning

- **Decision Logs**:
  - Audit trail of policy evaluations
  - **Impact**: HIGH - Compliance, debugging

- **Status API**:
  - Health monitoring, diagnostics
  - **Impact**: MEDIUM - Ops/monitoring

- **Discovery**:
  - Dynamic policy/bundle discovery
  - **Impact**: LOW - Large deployments

- **Plugins**:
  - Extensibility framework
  - **Impact**: LOW - Custom integrations

---

## 15. Expression Complexity

### ✅ Reaper HAS (2/6)
- Simple comparisons: `user.role == "admin"`
- Nested AND/OR/NOT

### ❌ Reaper MISSING (4/6)
- **Nested Comprehensions**:
  - Rego: `{u | users[u]; {r | u.roles[r]; r.active}}`
  - **Impact**: HIGH - Complex data transformations

- **Multi-variable Binding**:
  - Rego: `x, y in object` (iterate key-value pairs)
  - **Impact**: MEDIUM - Iterate maps

- **Conditional Expressions**:
  - Rego: `result := x if condition else y`
  - **Impact**: MEDIUM - Ternary operator

- **With Expressions**:
  - Override input/data: `expr with input.user as "admin"`
  - **Impact**: HIGH - Testing, what-if analysis

---

## 16. Data Model

### ✅ Reaper HAS
- DataStore with entities
- RBAC views (user_permission, role_users, resource_permissions)
- String interning

### ❌ Reaper MISSING
- **`data` Global**:
  - Rego: `data.users`, `data.roles`
  - All external data accessed via `data.`
  - **Impact**: HIGH - Different data model

- **`input` Global**:
  - Rego: `input.user`, `input.action`
  - All request data via `input.`
  - **Impact**: HIGH - Different request model

- **Nested Document Structure**:
  - Rego: Arbitrary depth: `data.org.team.user.permissions`
  - **Impact**: MEDIUM - Flat vs hierarchical

---

## 17. Error Handling

### ✅ Reaper HAS (1/3)
- Default deny on evaluation failure

### ❌ Reaper MISSING (2/3)
- **Undefined Handling**:
  - Rego: Undefined != false (three-valued logic)
  - **Impact**: HIGH - Different semantics

- **Strict Mode**:
  - `--strict-builtin-errors` flag
  - Built-in errors halt vs return undefined
  - **Impact**: MEDIUM - Error propagation control

---

## Gap Severity Analysis

### CRITICAL Gaps (Must Have for Rego Compatibility)
1. **Arrays** - Essential data structure
2. **Objects** - Essential data structure
3. **Sets** - Core Rego idiom
4. **`in` Operator** - Membership + iteration
5. **Set Comprehensions** - Core Rego pattern
6. **Local Variables** (`:=`) - Intermediate values
7. **Built-in Functions**:
   - Aggregates: `count()`, `sum()`, `max()`, `min()`
   - Strings: `concat()`, `contains()`, `split()`, `lower()`, `upper()`
   - Objects: `object.get()`, `object.keys()`
   - Time: `time.now_ns()`, `time.parse_ns()`
8. **Partial Sets/Objects** - Core Rego rule type
9. **Bracket Notation** - Dynamic key access
10. **Testing Framework** - `test_`, `with` keyword

### HIGH Priority Gaps (Important for Real Use Cases)
1. **User-Defined Functions** - Code reuse
2. **Array Comprehensions** - Data transformation
3. **Object Comprehensions** - Map building
4. **`every` Keyword** - Universal quantification
5. **JWT Built-ins** - Token validation (common)
6. **Regex Functions** - Pattern matching
7. **Set Operators** (`&`, `|`, `-`) - Combine sets
8. **Imports** - Modular policies
9. **Incremental Rules** - Policy composition
10. **Decision Logs** - Auditing

### MEDIUM Priority Gaps (Nice to Have)
1. **`else` Keyword** - Fallback rules
2. **Arithmetic Operators** - Math in policies
3. **Type Checking Functions** - Validation
4. **Encoding/Decoding** - Base64, JSON, YAML
5. **Networking Functions** - CIDR, IP validation
6. **Metadata** - Documentation
7. **`some` Keyword** - Explicit iteration
8. **HTTP Function** - External data
9. **Partial Evaluation** - Optimization
10. **Bundles** - Deployment

### LOW Priority Gaps (Specialized/Rare)
1. **GraphQL Functions** - Niche
2. **Semantic Versioning** - Niche
3. **Units Parsing** - Niche
4. **UUID Functions** - Niche
5. **Bits Operations** - Rare
6. **WASM Compilation** - We're already fast
7. **Plugins** - Extensibility
8. **Discovery** - Large deployments
9. **AWS Provider** - AWS-specific
10. **OPA Runtime** - Introspection

---

## Estimated Implementation Effort

### Phase 1: Core Data Structures (4-6 weeks)
- Arrays, Objects, Sets
- Bracket notation
- `in` operator
- Local variables (`:=`)
- Effort: **HIGH** (foundational changes)

### Phase 2: Comprehensions & Iteration (3-4 weeks)
- Set comprehensions
- Array comprehensions
- Object comprehensions
- `every` keyword
- Effort: **MEDIUM-HIGH**

### Phase 3: Essential Built-ins (6-8 weeks)
- Aggregates (6 functions)
- Strings (30+ functions)
- Objects (11 functions)
- Time (11 functions)
- Type checking (7 functions)
- Effort: **HIGH** (many functions)

### Phase 4: Functions & Modules (3-4 weeks)
- User-defined functions
- Imports
- Partial sets/objects
- Effort: **MEDIUM**

### Phase 5: Testing & Tooling (2-3 weeks)
- Test framework
- `with` keyword
- `trace()` function
- Effort: **MEDIUM**

### Phase 6: Advanced Built-ins (8-10 weeks)
- JWT (16 functions)
- Regex (7 functions)
- Encoding (14 functions)
- Crypto (16 functions)
- Networking (7 functions)
- HTTP (1 function)
- Effort: **HIGH**

### Phase 7: Advanced Features (4-5 weeks)
- Decision logs
- Bundles
- Metadata/schemas
- Effort: **MEDIUM**

**TOTAL ESTIMATED**: **30-40 weeks** for full Rego compatibility

---

## Alternative Approaches

### Option 1: Full Rego Compatibility
- **Effort**: 30-40 weeks
- **Pros**: Drop-in OPA replacement, full feature parity
- **Cons**: Massive effort, may lose performance edge
- **Recommendation**: ⚠️ Only if targeting OPA migration market

### Option 2: Rego-to-Reaper Transpiler
- **Effort**: 12-16 weeks
- **Pros**: Support Rego syntax, maintain Reaper backend
- **Cons**: Won't support all Rego features
- **Recommendation**: ✅ Good middle ground

### Option 3: Extend Reaper DSL Selectively
- **Effort**: 8-12 weeks (Phase 1-3 only)
- **Pros**: Keep simplicity, add essential features
- **Cons**: Not Rego-compatible
- **Recommendation**: ✅ Best for current users

### Option 4: Hybrid (Cedar + Selective Rego)
- **Effort**: 6-8 weeks (Cedar) + 4-6 weeks (select Rego features)
- **Pros**: AWS compatibility + some Rego power
- **Cons**: Two languages to maintain
- **Recommendation**: ✅ Best overall strategy

---

## Recommendations

### Immediate Priorities (Next 3 Months)

**1. Cedar Language Support** (3 weeks)
- AWS Verified Permissions compatibility
- Established standard
- Simpler than Rego
- High user demand

**2. Essential Reaper DSL Extensions** (4-6 weeks)
- Arrays, Objects (data structures)
- `in` operator (membership/iteration)
- Local variables (`:=`)
- Basic built-ins:
  - Aggregates: `count()`, `sum()`, `max()`, `min()`
  - Strings: `concat()`, `contains()`, `split()`
  - Objects: `object.get()`, `object.keys()`

**3. Time-Based Policies** (2 weeks)
- `time.now_ns()`, `time.parse_ns()`
- Expiration, time windows
- Common use case

### Medium-Term (3-6 Months)

**4. Basic Comprehensions** (3-4 weeks)
- Set comprehensions
- Array comprehensions

**5. User-Defined Functions** (2-3 weeks)
- Code reuse
- Policy composition

**6. Testing Framework** (2 weeks)
- `test_` rules
- `with` keyword for mocking

### Long-Term (6-12 Months)

**7. Rego Transpiler** (12-16 weeks)
- Parse Rego AST
- Transpile to Reaper IR
- Support subset of Rego features

**8. Advanced Built-ins** (as needed)
- JWT validation
- Regex matching
- Encoding/decoding

---

## Conclusion

**Current Gap**: Reaper DSL is ~10-15% of Rego's functionality

**Critical Missing Features**:
1. Arrays, Objects, Sets (data structures)
2. Comprehensions (core idiom)
3. Built-in functions (~200 missing)
4. Testing framework
5. Modules/imports

**Best Strategy**:
1. **Immediate**: Add Cedar support (industry standard)
2. **Short-term**: Extend Reaper DSL with essentials (arrays, objects, basic built-ins)
3. **Medium-term**: Build Rego transpiler for migration path
4. **Long-term**: Add advanced features based on user demand

**Estimated Effort**:
- Cedar: 3 weeks
- Essential DSL extensions: 6 weeks
- Basic transpiler: 12 weeks
- **Total**: ~21 weeks for solid language support

This provides **pragmatic Rego compatibility** without the **30-40 week full implementation**.
