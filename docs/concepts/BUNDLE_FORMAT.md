# Reaper Bundle Format

This document describes the binary bundle formats used by Reaper for policy distribution.

## Bundle Types

### .rbb - Reaper Binary Bundle

Single-policy bundle format optimized for fast loading.

### .rpp - Reaper Policy Package

Multi-policy package with pre-compilation hints (planned).

## .rbb Format Specification

### File Structure

```
┌────────────────────────────────────────┐
│           Magic Header (4 bytes)        │  "REAP"
├────────────────────────────────────────┤
│           Version (4 bytes)             │  Little-endian u32
├────────────────────────────────────────┤
│         Metadata Length (4 bytes)       │  Little-endian u32
├────────────────────────────────────────┤
│              Metadata                   │  JSON (variable)
├────────────────────────────────────────┤
│          Policy Length (4 bytes)        │  Little-endian u32
├────────────────────────────────────────┤
│            Policy Data                  │  Bincode (variable)
├────────────────────────────────────────┤
│            Checksum (32 bytes)          │  SHA-256
└────────────────────────────────────────┘
```

### Metadata Format

```json
{
  "version": 1,
  "policy_name": "rbac-simple",
  "policy_version": "1.0.0",
  "compiled_at": "2024-01-15T10:30:00Z",
  "compiler_version": "0.1.0",
  "source_checksum": "sha256:...",
  "features": ["string_interning", "compiled_conditions"]
}
```

### Version History

| Version | Changes |
|---------|---------|
| 1 | Initial format |

## Creating Bundles

### Using the CLI

```bash
# Compile a single policy to .rbb
reaper compile policy.reap -o policy.rbb

# Compile with optimizations
reaper compile policy.reap -o policy.rbb --optimize

# View bundle metadata
reaper bundle info policy.rbb
```

### Programmatic Creation

```rust
use policy_engine::{ReaperPolicy, PolicyBundle};

// Load policy from .reap file
let policy = ReaperPolicy::from_file_auto("policy.reap")?;

// Compile to bundle
let bundle = policy.compile_to_bundle()?;

// Save bundle
std::fs::write("policy.rbb", bundle.to_bytes()?)?;
```

## Loading Bundles

### Agent API

```bash
# Deploy bundle to agent
curl -X POST http://localhost:8080/api/v1/bundles/deploy \
  -H "Content-Type: application/octet-stream" \
  --data-binary @policy.rbb
```

### CLI Deployment

```bash
# Deploy bundle with data
reaper bundle deploy policy.rbb --data entities.json
```

### Programmatic Loading

```rust
use policy_engine::PolicyBundle;

// Load from bytes
let bytes = std::fs::read("policy.rbb")?;
let bundle = PolicyBundle::from_bytes(&bytes)?;

// Verify checksum
if !bundle.verify_checksum() {
    return Err("Checksum mismatch");
}

// Access policy
let policy = bundle.policy();
```

## Checksum Verification

All bundles include a SHA-256 checksum for integrity verification.

### Verification Flow

1. Extract checksum from bundle tail (32 bytes)
2. Calculate SHA-256 of bundle (excluding checksum)
3. Compare calculated vs stored checksum
4. Reject if mismatch

### Example

```rust
use sha2::{Sha256, Digest};

fn verify_bundle(data: &[u8]) -> bool {
    if data.len() < 32 {
        return false;
    }

    let (content, stored_checksum) = data.split_at(data.len() - 32);
    let mut hasher = Sha256::new();
    hasher.update(content);
    let calculated = hasher.finalize();

    calculated.as_slice() == stored_checksum
}
```

## Bundle Deployment

### Deployment Modes

1. **Hot Reload**: Deploy without restart (default)
   - Atomic policy replacement
   - Zero downtime
   - Old policy garbage collected

2. **Force Deploy**: Override version checks
   - Useful for rollbacks
   - Requires `--force` flag

### Deployment Response

```json
{
  "policy_id": "uuid",
  "version": "1.0.0",
  "deployed_at": "2024-01-15T10:30:00Z",
  "bundle_hash": "sha256:abcd1234..."
}
```

## Version Management

### Versioning Scheme

Bundles use semantic versioning (semver):

- **Major**: Breaking policy changes
- **Minor**: New rules or features
- **Patch**: Bug fixes

### Version Checks

By default, agents reject bundles with lower versions. Override with `force: true`.

## Best Practices

### Development

1. Use `.reap` files for source control
2. Compile to `.rbb` for deployment
3. Store checksums in deployment records
4. Version bundles with semver

### Production

1. Always verify checksums before deployment
2. Keep previous bundle for rollback
3. Monitor deployment metrics
4. Use staged rollouts for critical policies

### Security

1. Sign bundles for production (planned)
2. Encrypt sensitive policy content (planned)
3. Validate bundle source
4. Audit all deployments

## Error Handling

### Common Errors

| Error | Cause | Solution |
|-------|-------|----------|
| `InvalidMagic` | Not a valid .rbb file | Check file format |
| `UnsupportedVersion` | Bundle version too new | Update agent |
| `ChecksumMismatch` | File corrupted | Re-download |
| `DeserializationError` | Invalid policy data | Recompile |

### Recovery

1. Keep last known good bundle
2. Automatic rollback on deployment failure
3. Alert on repeated failures

## Future Enhancements

### .rpp Package Format

Multi-policy packages with:

- Multiple policies in one file
- Shared entity schemas
- Pre-interned strings
- Compiled regex cache
- Dependency declarations

### Bundle Signing

Cryptographic signatures for:

- Publisher verification
- Tamper detection
- Audit trails

### Encryption

Optional encryption for:

- Sensitive policy rules
- Secret conditions
- Compliance requirements

## CLI Reference

```bash
# Compile policy
reaper compile <input> -o <output.rbb> [--optimize]

# Bundle info
reaper bundle info <bundle.rbb>

# Deploy bundle
reaper bundle deploy <bundle.rbb> [--data <entities.json>] [--force]

# Validate bundle
reaper bundle validate <bundle.rbb>
```

## Related Documentation

- [Event-Driven Loading](EVENT_DRIVEN_LOADING.md) - Bundle sync
- [Operations Guide](../deployment/OPERATIONS_GUIDE.md) - Deployment
- [CLI Reference](../reference/) - Full CLI documentation
