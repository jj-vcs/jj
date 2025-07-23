# Rerere (Reuse Recorded Resolution) Design

## Overview

Rerere ("reuse recorded resolution") automatically records how merge conflicts were resolved and applies the same resolution when identical conflicts appear again. This is particularly useful when rebasing branches or re-merging changes through different paths.

## Motivation

While Jujutsu has excellent conflict handling with first-class conflict objects and automatic propagation during rebasing, it lacks cross-branch resolution memory. When the same logical conflict appears in different contexts (e.g., cherry-picking changes or merging through different paths), users must resolve it manually each time.

## Design Principles

1. **Transparent Operation**: Rerere should work automatically without user intervention
2. **Content-Based Matching**: Conflicts are identified by their content, not by file paths or commit IDs
3. **Path Independence**: The same conflict in different files should reuse the same resolution
4. **Multi-way Support**: Support Jujutsu's n-way conflicts, not just 3-way merges

## Architecture

### Conflict Identification

Conflicts are identified by normalizing and hashing their content:
- Conflict sides are sorted to ensure order independence
- File paths are excluded from the hash
- Only the actual conflicting content is considered
- Uses BLAKE2b-512 for fast, secure content hashing

This allows the same logical conflict to be recognized regardless of:
- Which file it appears in
- The order of conflict sides
- The specific commits involved

### Integration Points

Rerere integrates at two key points in Jujutsu's architecture:

1. **During Conflict Resolution**: When conflicts are resolved (either manually or via merge tools), the resolution is automatically recorded
2. **During Tree Merging**: When new conflicts are created, the cache is checked for matching resolutions

### Storage Strategy

Resolutions are stored in a local cache within the repository. The cache uses content-addressed storage where:
- Keys are derived from normalized conflict content
- Values are the resolved content
- Entries expire after a configurable period (default: 60 days)

### File Format

The resolution cache uses a simple, robust file format:

```
<conflict-hash>/
  ├── conflict      # Normalized conflict (for debugging)
  └── resolution    # The resolved content
```

The normalized conflict format ensures consistent hashing:
```
CONFLICT:3
SIDE_START
<content of side 1>
SIDE_END
SIDE_START
<content of side 2>
SIDE_END
SIDE_START
<content of side 3>
SIDE_END
```

The conflict hash (used as the directory name) is derived by:
1. Normalizing the conflict into the format shown above
2. Computing BLAKE2b-512 hash of the normalized content
3. Using the hex encoding of the hash as the directory name

Key properties:
- **Content-addressed**: Directory name is the conflict's content hash
- **Self-contained**: Each resolution is independent
- **Debuggable**: Can inspect with standard tools (cat, diff, etc.)
- **Atomic**: Directories are created atomically via rename
- **Timestamp-based GC**: Uses filesystem mtime for expiry

## User Experience

### Basic Workflow

1. User encounters a conflict and resolves it
2. Jujutsu automatically records the resolution
3. When the same conflict appears later, it's automatically resolved
4. User sees a message: "Applied N cached conflict resolutions"

### Configuration

```toml
[rerere]
enabled = true              # Enable rerere (default: false)
```

## Design Decisions

### Why Content-Based Matching?

Content-based matching ensures that logically identical conflicts are recognized regardless of context. This is more robust than Git's approach which can be confused by different file paths or commit histories.

### Why No Manual Commands?

Unlike Git's rerere, Jujutsu's implementation provides no manual commands (`forget`, `clear`, etc.). This aligns with Jujutsu's philosophy of automatic operation. Cache maintenance happens through:
- Automatic expiry of old entries
- Standard garbage collection (`jj util gc`)

### Why Store in the Repository?

Storing resolutions in the repository (rather than globally) ensures:
- Resolutions are project-specific
- No cross-project contamination
- Natural cleanup when repositories are deleted

## Comparison with Git's Rerere

| Aspect | Git | Jujutsu |
|--------|-----|---------|
| Conflict Detection | Path + content based | Pure content based |
| Manual Commands | Many (`forget`, `clear`, `diff`, etc.) | None |
| Multi-way Conflicts | No | Yes |
| Auto-apply | Optional | Default |
| Storage Location | `.git/rr-cache` | `.jj/repo/resolution_cache` |

## Future Considerations

- **Sharing Resolutions**: Could allow sharing resolution cache between team members
- **Merge Tool Integration**: Could provide rerere hints to external merge tools
- **Statistics**: Could track rerere effectiveness metrics

## Security Considerations

- Resolution cache is local-only and never shared
- Cache entries are validated before application
- Malformed cache entries are ignored rather than causing failures
