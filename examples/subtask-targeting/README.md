# Subtask Targeting Example

This example demonstrates **subtask targeting** - the ability to run a specific
subtask from a `foreach` task without running all siblings.

## The Problem This Solves

Previously, running `otto install:typescript` would also run `install:golang` and
`install:python` because the parser expanded all subtasks. Now, you can target
exactly the subtask you need.

## Commands to Try

### Run ONE specific subtask

```bash
otto install:typescript
```

Output:
```
[install:typescript] === Installing typescript toolchain ===
[install:typescript]   Downloading typescript...
[install:typescript]   Configuring typescript...
[install:typescript]   typescript installed successfully!
```

Only `install:typescript` runs - not `install:golang` or `install:python`.

### Run ALL subtasks via parent

```bash
otto install
```

Output shows all three running in parallel:
```
[install:golang] === Installing golang toolchain ===
[install:python] === Installing python toolchain ===
[install:typescript] === Installing typescript toolchain ===
...
```

### Task depending on specific subtask

```bash
otto deploy
```

The `deploy` task has `before: [install:typescript]`. Only that specific
subtask runs as a dependency:

```
[install:typescript] === Installing typescript toolchain ===
...
[deploy] === Deploying TypeScript application ===
[deploy]   (Note: only install:typescript ran, not install:golang or install:python)
```

### View the task graph

```bash
otto Graph
```

Shows the improved graph notation:
```
├─ ci
   ├─ deploy
      └─ install:{golang,python,typescript}
   ├─ test-backend
      ├─ install:{golang,python,typescript}
   └─ install:{golang,python,typescript}
```

Notice `install:{golang,python,typescript}` instead of `install:* [3 items]`.

## Key Features

1. **Subtask targeting**: `otto install:typescript` runs only that subtask
2. **Parent runs all**: `otto install` runs all subtasks in parallel
3. **Specific dependencies**: Tasks can depend on specific subtasks like `install:typescript`
4. **Improved graph display**: Shows `{item1,item2,...}` for small item lists
