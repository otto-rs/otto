# Test Database Isolation

## Problem

Integration tests were polluting the production database (`~/.otto/otto.db`) with test runs from temporary directories (e.g., `/tmp/.tmpXXXXXX`). This caused:
- Cluttered history output showing test runs alongside real runs
- Difficult to distinguish real usage from test execution
- Potential test interference with production data

## Solution

Added support for `OTTO_DB_PATH` environment variable to override the default database location.

### Changes

1. **Database Manager Enhancement** (`src/executor/state/db.rs`)
   - Modified `default_db_path()` to check for `OTTO_DB_PATH` environment variable
   - Falls back to `~/.otto/otto.db` if not set
   - Enables test isolation without affecting production behavior

2. **Integration Test Updates**
   - `tests/execution_context_integration_test.rs`: Added `setup_test_db()` helper
   - `tests/file_dependencies_integration_test.rs`: Updated `TestFixture::new()` and standalone tests
   - `tests/executor_test.rs`: Added `setup_test_db()` helper to all 6 tests
   - `tests/builtin_commands_test.rs`: Added `OTTO_DB_PATH` env var to all command invocations
   - All integration tests now use isolated test databases in their temp directories

3. **Unit Test Updates in Source Files**
   - `src/executor/action.rs`: Added `setup_test_db()` helper to 3 tests
   - `src/executor/scheduler.rs`: Added `setup_test_db()` helper to 12 tests
   - `src/executor/workspace.rs`: Added `setup_test_db()` helper to 3 tests
   - All unit tests that create workspaces now use isolated databases

### Usage

#### For Testing
```rust
// In test setup
let temp_dir = TempDir::new()?;
let db_path = temp_dir.join("test_otto.db");
unsafe {
    std::env::set_var("OTTO_DB_PATH", &db_path);
}
```

#### For Production Override (if needed)
```bash
export OTTO_DB_PATH=/path/to/custom/otto.db
otto history
```

## Verification

After the fix:
- 69 test runs removed from production database
- Tests continue to pass with isolated databases
- No new `/tmp` entries appear in production database after test runs
- History command shows only legitimate runs

## Notes

- The `unsafe` block for `set_var` is acceptable in tests because:
  - Tests control their execution environment
  - Tests are isolated from each other
  - Environment variable is set before any StateManager initialization
- Production code never uses `set_var`, only reads the environment variable

