# Sampler Test Coverage Plan

## Current State

| Module | Coverage | Has Unit Tests | Has Integration Tests |
|--------|----------|----------------|----------------------|
| `auditioner.rs` | 0% | ❌ | ✓ (2 tests) |
| `handle.rs` | 0% | ❌ | ✓ (4 tests) |
| `butler/refill.rs` | 3% | ❌ | ✓ (indirect) |
| `butler/capture.rs` | 7% | ❌ | ✓ (1 test) |
| `butler/loops.rs` | 13% | ❌ | ✓ (2 tests) |
| `sampler/node.rs` | 24% | ❌ | ✓ (many) |
| `stream_builder.rs` | 24% | ❌ | ✓ (indirect) |
| `butler/thread.rs` | 27% | ❌ | ✓ (indirect) |

## Analysis

### 1. `auditioner.rs` (0/81 lines)

**What it does**: Preview player for quick file auditioning from browser panel.

**Why 0% unit test coverage**:
- Needs real `SamplerSystem` with butler thread
- Integration tests exist but tarpaulin doesn't count them

**Testing approach**: Unit tests not practical - requires full system.
**Recommendation**: Expand integration tests in `sampler_integration.rs`:
- Test `preview()` with various file sizes (in-memory vs streaming threshold)
- Test `stop()` during playback
- Test rapid preview switching
- Test `set_volume()` / `set_speed()` actually affect output

---

### 2. `handle.rs` (0/87 lines)

**What it does**: Fluent handle wrapping `SamplerSystem` operations.

**Why 0% unit test coverage**:
- Thin wrapper, delegates to `SamplerSystem`
- Integration tests cover it via engine API

**Testing approach**: Can add unit tests for disabled handle behavior.
**Recommendation**:
- Unit test: `SamplerHandle::new(None)` returns no-op handle
- Unit test: `disabled()` builder methods return safely
- Integration tests already cover enabled path

---

### 3. `butler/refill.rs` (5/190 lines)

**What it does**: Core disk streaming logic - refills ring buffers from disk.

**Why low coverage**:
- `calculate_varifill_chunk()` - pure function, easy to unit test
- `refill_all_streams()` - needs full butler setup
- `refill_single_stream()` - needs file I/O

**Testing approach**:
- Unit test `calculate_varifill_chunk()` with various inputs
- Integration tests for actual streaming behavior

**Recommendation**:
```rust
#[test]
fn test_varifill_chunk_empty_buffer() {
    // buffer_fill=0.0 (empty) should give larger chunks
    let chunk = calculate_varifill_chunk(0.0, 4096, 10_000_000.0, 1.0);
    assert!(chunk > 4096); // urgency increases chunk
}

#[test]
fn test_varifill_chunk_full_buffer() {
    // buffer_fill=1.0 (full) should give smaller chunks
    let chunk = calculate_varifill_chunk(1.0, 4096, 10_000_000.0, 1.0);
    assert!(chunk <= 4096);
}

#[test]
fn test_varifill_chunk_high_speed() {
    // speed=2.0 should give larger chunks
    let normal = calculate_varifill_chunk(0.5, 4096, 10_000_000.0, 1.0);
    let fast = calculate_varifill_chunk(0.5, 4096, 10_000_000.0, 2.0);
    assert!(fast > normal);
}
```

---

### 4. `butler/capture.rs` (2/30 lines)

**What it does**: Handles capture buffer draining to disk during recording.

**Why low coverage**:
- Needs active capture session with audio flowing
- Integration test creates session but doesn't drain much

**Testing approach**: Integration test with longer recording.
**Recommendation**: Expand `test_sampler_capture_session_creation` to:
- Actually record audio for a few seconds
- Verify file size grows
- Verify audio content matches input

---

### 5. `butler/loops.rs` (12/89 lines)

**What it does**: Loop point detection, crossfade sample capture.

**Why low coverage**:
- `check_loop_point()` needs active streaming with loop set
- `capture_fadeout_samples()` / `capture_fadein_samples()` need buffer state

**Testing approach**: Can unit test capture functions.
**Recommendation**:
- Unit test `capture_fadeout_samples()` with mock stream state
- Unit test `capture_fadein_samples()` with cache hit/miss
- Integration tests already cover loop crossfade

---

### 6. `sampler/node.rs` (91/381 lines, 24%)

**What it does**: Main audio processing node - the `SamplerNode` that renders audio.

**Why 24% coverage**:
- Integration tests cover basic playback
- Many edge cases not covered (varispeed, reverse, PDC, etc.)

**Testing approach**: Expand integration tests.
**Recommendation**:
- Test various sample rates (SRC paths)
- Test mono→stereo expansion
- Test seeking during playback
- Test buffer underrun handling

---

## Priority Order

1. **`butler/refill.rs`** - Add unit tests for `calculate_varifill_chunk()` (easy win, pure function)
2. **`handle.rs`** - Add unit tests for disabled handle (easy, no dependencies)
3. **`butler/loops.rs`** - Add unit tests for capture functions
4. **`auditioner.rs`** - Expand integration tests
5. **`butler/capture.rs`** - Expand integration test with real recording
6. **`sampler/node.rs`** - More integration test edge cases

## Summary

- **Easy unit tests** (no deps): `refill.rs::calculate_varifill_chunk`, `handle.rs` disabled path
- **Medium unit tests** (mock deps): `loops.rs` capture functions
- **Integration only**: `auditioner.rs`, `capture.rs`, `node.rs`, `thread.rs`
