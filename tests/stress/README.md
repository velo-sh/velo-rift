# Stress & Performance Tests

These tests are resource-intensive and should NOT run on every CI build.

## Test Categories

| Prefix | Type | CI Schedule |
|--------|------|-------------|
| `stress_` | Stress tests (memory, concurrent) | Weekly/Manual |
| `bench_` | Performance benchmarks | Pre-release |
| `soak_` | Long-running (hours) | Manual only |

## Running

```bash
# Run all stress tests
for f in tests/stress/*.sh; do bash "$f"; done

# Run specific benchmark
bash tests/stress/bench_stat_throughput.sh
```

## Current Tests

- `stress_large_file_4gb.sh` - >4GB file handling (sparse file)
- `stress_concurrent_10proc.sh` - 10 parallel processes
- `bench_stat_throughput.sh` - IPC throughput benchmark
