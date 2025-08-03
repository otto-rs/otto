# Action Key Restructuring Plan

## Overview

This document outlines the plan to restructure Otto's YAML task action specification from a single `action` string field to language-specific keys (`bash` and `python`). This change eliminates the need for user-provided shebangs and provides better type safety and validation.

## Current State vs Target State

### Current YAML Structure
```yaml
tasks:
  example:
    action: |
      #!/bin/bash
      echo "Hello World"

  python_task:
    action: |
      #!/usr/bin/env python3
      print("Hello World")
```

### Target YAML Structure
```yaml
tasks:
  example:
    bash: |
      echo "Hello World"

  python_task:
    python: |
      print("Hello World")
```

## Benefits

1. **Cleaner YAML**: No more shebangs in user code
2. **Type Safety**: Explicit action types prevent ambiguity
3. **Better Validation**: Can validate at parse time rather than execution time
4. **Consistency**: All bash actions get consistent shebang, same for python
5. **Extensibility**: Easy to add new language types in the future
6. **User Experience**: Less boilerplate, clearer intent

## Architecture Changes

### Data Flow Overview
```
Current:  YAML → String → Task → ActionProcessor (detect language) → ProcessedAction
Target:   YAML → ActionSpec → Task → ActionProcessor → ProcessedAction
```

### Type System
```rust
// Parse-time: User input from YAML
pub enum ActionSpec {
    Bash(String),     // Raw user script
    Python(String),   // Raw user script
}

// Runtime: Fully processed and ready to execute
pub enum ProcessedAction {
    Bash { path: PathBuf, script: String, hash: String },
    Python { path: PathBuf, script: String, hash: String },
}
```

## Detailed Implementation Plan

### Phase 1: Core Structure Changes

#### 1.1 TaskSpec Changes (`src/cfg/task.rs`)

**Remove:**
```rust
#[serde(default, deserialize_with = "deserialize_script")]
pub action: String,
```

**Add:**
```rust
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionSpec {
    Bash(String),
    Python(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct TaskSpec {
    // ... existing fields ...

    #[serde(flatten)]
    pub action: Option<ActionSpec>,
}
```

**Custom Deserializer:**
```rust
impl<'de> Deserialize<'de> for ActionSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ActionVisitor;

        impl<'de> Visitor<'de> for ActionVisitor {
            type Value = ActionSpec;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("either 'bash' or 'python' key with script content")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut bash_content: Option<String> = None;
                let mut python_content: Option<String> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "bash" => {
                            if bash_content.is_some() {
                                return Err(serde::de::Error::duplicate_field("bash"));
                            }
                            let content: String = map.next_value()?;
                            bash_content = Some(deserialize_script_content(content)?);
                        }
                        "python" => {
                            if python_content.is_some() {
                                return Err(serde::de::Error::duplicate_field("python"));
                            }
                            let content: String = map.next_value()?;
                            python_content = Some(deserialize_script_content(content)?);
                        }
                        _ => {
                            // Skip unknown fields
                            let _: serde::de::IgnoredAny = map.next_value()?;
                        }
                    }
                }

                match (bash_content, python_content) {
                    (Some(bash), None) => Ok(ActionSpec::Bash(bash)),
                    (None, Some(python)) => Ok(ActionSpec::Python(python)),
                    (Some(_), Some(_)) => Err(serde::de::Error::custom(
                        "task cannot have both 'bash' and 'python' actions"
                    )),
                    (None, None) => Err(serde::de::Error::custom(
                        "task must have either 'bash' or 'python' action"
                    )),
                }
            }
        }

        deserializer.deserialize_map(ActionVisitor)
    }
}

// Reuse existing deserialize_script logic for content processing
fn deserialize_script_content(s: String) -> Result<String, serde::de::Error> {
    // Extract and reuse the existing deserialize_script implementation
    let lines: Vec<&str> = s.lines().collect();

    // Find minimum indentation (ignoring empty lines)
    let min_indent = lines.iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);

    // Remove common indentation from each line
    let dedented: Vec<String> = lines.iter()
        .map(|line| {
            if line.len() > min_indent {
                line[min_indent..].to_string()
            } else {
                line.to_string()
            }
        })
        .collect();

    // Join lines and trim any leading/trailing empty lines
    let result = dedented.join("\n");
    Ok(result.trim_start().trim_end().to_string())
}
```

**Validation:**
```rust
impl TaskSpec {
    pub fn validate(&self) -> Result<()> {
        match &self.action {
            Some(ActionSpec::Bash(content)) if content.trim().is_empty() => {
                Err(eyre!("Bash action cannot be empty"))
            }
            Some(ActionSpec::Python(content)) if content.trim().is_empty() => {
                Err(eyre!("Python action cannot be empty"))
            }
            None => Err(eyre!("Task must have either 'bash' or 'python' action")),
            _ => Ok(())
        }
    }
}
```

#### 1.2 Task Runtime Structure (`src/executor/task.rs`)

**Update Task struct:**
```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub name: String,
    pub task_deps: Vec<String>,
    pub file_deps: Vec<String>,
    pub output_deps: Vec<String>,
    pub envs: HashMap<String, String>,
    pub values: HashMap<String, Value>,
    pub action: ActionSpec,  // Changed from String
    pub hash: String,
}
```

**Update Task creation methods:**
```rust
impl Task {
    pub fn new(
        name: String,
        task_deps: Vec<String>,
        file_deps: Vec<String>,
        output_deps: Vec<String>,
        envs: HashMap<String, String>,
        values: HashMap<String, Value>,
        action: ActionSpec,  // Changed from String
    ) -> Self {
        // Hash calculation needs to be updated to handle ActionSpec
        let hash = calculate_hash(&action);
        Self {
            name,
            task_deps,
            file_deps,
            output_deps,
            envs,
            values,
            action,
            hash,
        }
    }

    pub fn from_task_with_cwd_and_global_envs(
        task_spec: &TaskSpec,
        cwd: &std::path::Path,
        global_envs: &HashMap<String, String>
    ) -> Result<Self> {
        // ... existing logic ...

        let action = task_spec.action.as_ref()
            .ok_or_else(|| eyre!("Task must have an action"))?
            .clone();

        Ok(Self::new(name, task_deps, file_deps, output_deps, evaluated_envs, values, action))
    }
}

// Update hash calculation to work with ActionSpec
fn calculate_hash(action: &ActionSpec) -> String {
    let content = match action {
        ActionSpec::Bash(script) => format!("bash:{}", script),
        ActionSpec::Python(script) => format!("python:{}", script),
    };

    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result)[..8].to_string()
}
```

#### 1.3 Action Processing Changes (`src/executor/action.rs`)

**Update ActionProcessor::process method:**
```rust
impl ActionProcessor {
    pub fn process(&self, action_spec: &ActionSpec, task: &Task) -> Result<ProcessedAction> {
        match action_spec {
            ActionSpec::Bash(user_script) => {
                let processor = BashProcessor::new(self.workspace.clone(), &self.task_name);
                processor.create_builtins()?;
                let script = self.build_script(&processor, user_script, task)?;
                let path = self.write_script(&processor, &script)?;
                let hash = self.calculate_hash(&script)?;
                Ok(ProcessedAction::Bash { path, script, hash })
            }
            ActionSpec::Python(user_script) => {
                let processor = PythonProcessor::new(self.workspace.clone(), &self.task_name);
                processor.create_builtins()?;
                let script = self.build_script(&processor, user_script, task)?;
                let path = self.write_script(&processor, &script)?;
                let hash = self.calculate_hash(&script)?;
                                 Ok(ProcessedAction::Python { path, script, hash })
            }
        }
    }
}
```

**Update build_script method:**
```rust
fn build_script<T: ScriptProcessor>(&self, processor: &T, user_action: &str, task: &Task) -> Result<String> {
    // Remove shebang extraction logic - user_action is now clean
    let prologue = processor.generate_prologue(&task.task_deps, task)?;
    let epilogue = processor.generate_epilogue()?;

    // Build script: prologue + user content + epilogue
    let script = format!("{}\n{}\n{}", prologue, user_action, epilogue);
    Ok(script)
}
```

**Update ScriptProcessor implementations:**

**BashProcessor:**
```rust
impl ScriptProcessor for BashProcessor {
    fn generate_prologue(&self, dependencies: &[String], task: &Task) -> Result<String> {
        let env_section = self.generate_bash_env_section(task);
        let input_section = self.generate_bash_input_section(dependencies);
        let param_section = self.generate_bash_param_section(task);

        let prologue = format!(r#"#!/bin/bash
# Otto-generated bash prologue
set -euo pipefail

declare -A OTTO_INPUT
declare -A OTTO_OUTPUT

# Set Otto environment variables
export OTTO_TASK_DIR="$(dirname "$0")"

# Source Otto builtins
source "$(dirname "$0")/builtins.sh"

{env_section}
{input_section}
{param_section}"#,
            env_section = env_section,
            input_section = input_section,
            param_section = param_section
        );
        Ok(prologue)
    }

    // ... rest unchanged
}
```

**PythonProcessor:**
```rust
impl ScriptProcessor for PythonProcessor {
    fn generate_prologue(&self, dependencies: &[String], task: &Task) -> Result<String> {
        let env_section = self.generate_python_env_section(task);
        let input_section = self.generate_python_input_section(dependencies);
        let param_section = self.generate_python_param_section(task);

        let prologue = format!(r#"#!/usr/bin/env python3
# Otto-generated python prologue
import json
import os
import glob
import sys

# Set Otto environment variables
os.environ['OTTO_TASK_DIR'] = os.path.dirname(__file__)

# Import Otto builtins
import importlib.util
builtins_path = os.path.join(os.path.dirname(__file__), 'builtins.py')
spec = importlib.util.spec_from_file_location("otto_builtins", builtins_path)
otto_builtins = importlib.util.module_from_spec(spec)
spec.loader.exec_module(otto_builtins)

# Make builtin functions available globally
otto_get_input = otto_builtins.otto_get_input
otto_set_output = otto_builtins.otto_set_output
otto_deserialize_input = otto_builtins.otto_deserialize_input
otto_serialize_output = otto_builtins.otto_serialize_output

OTTO_INPUT = {{}}
OTTO_OUTPUT = {{}}

{env_section}
{input_section}
{param_section}"#,
            env_section = env_section,
            input_section = input_section,
            param_section = param_section
        );
        Ok(prologue)
    }

    // ... rest unchanged
}
```

#### 1.4 Scheduler Changes (`src/executor/scheduler.rs`)

**Update task execution:**
```rust
// Process the user's action script with Otto enhancements
let action_processor = ActionProcessor::new(workspace.clone(), &task_name)?;
let processed_action = action_processor.process(&task.action, &task)?;  // Changed parameter

 // Extract script path and determine interpreter
 let (script_path, interpreter) = match processed_action {
     ProcessedAction::Bash { path, .. } => (path, "bash"),
     ProcessedAction::Python { path, .. } => (path, "python3"),
 };
```

#### 1.5 Parser Changes (`src/cli/parser.rs`)

**Update Task creation calls:**
```rust
// Ensure all Task::new() and Task::from_task_*() calls are updated
// to handle the new ActionSpec parameter
```

### Phase 2: Error Handling and Validation

#### 2.1 Enhanced Error Messages

```rust
impl fmt::Display for ActionSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionSpec::Bash(_) => write!(f, "bash action"),
            ActionSpec::Python(_) => write!(f, "python action"),
        }
    }
}

// Custom error types for better diagnostics
#[derive(Debug)]
pub enum ActionError {
    MissingAction,
    EmptyAction(String),  // language type
    ConflictingActions,
    InvalidSyntax(String),
}
```

#### 2.2 Migration Support (Temporary)

```rust
// Add temporary support for old format with helpful error messages
impl TaskSpec {
    pub fn check_legacy_action(&self) -> Result<()> {
        // This would be implemented to detect old 'action:' usage
        // and provide migration guidance
        Ok(())
    }
}
```

### Phase 3: Testing and Examples

#### 3.1 Unit Tests

**Update existing tests in `src/executor/action.rs`:**
```rust
#[tokio::test]
async fn test_bash_action_processing() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let workspace = Arc::new(Workspace::new(temp_dir.path().to_path_buf()).await?);
    workspace.init().await?;

    let processor = ActionProcessor::new(workspace.clone(), "test_task")?;

    // Create test task with ActionSpec
    let action_spec = ActionSpec::Bash("echo \"${greeting} world\"".to_string());

    let mut task_envs = HashMap::new();
    task_envs.insert("greeting".to_string(), "hello".to_string());

    let mut task_values = HashMap::new();
    task_values.insert("greeting".to_string(), Value::Item("hello".to_string()));

    let task = Task::new(
        "test_task".to_string(),
        vec!["dep_task".to_string()],
        vec![],
        vec![],
        task_envs,
        task_values,
        action_spec,
    );

    // Process the action
    let result = processor.process(&task.action, &task)?;

    // Verify the result
    match result {
        ProcessedAction::Bash { path, script, hash } => {
            assert!(path.exists());
            assert!(script.contains("#!/bin/bash"));
            assert!(script.contains("declare -A OTTO_INPUT"));
            assert!(script.contains("echo \"${greeting} world\""));
            // ... other assertions
        },
        _ => panic!("Expected Bash variant"),
    }

    Ok(())
}

 #[tokio::test]
 async fn test_python_action_processing() -> Result<()> {
     let temp_dir = TempDir::new()?;
     let workspace = Arc::new(Workspace::new(temp_dir.path().to_path_buf()).await?);
     workspace.init().await?;

     let processor = ActionProcessor::new(workspace.clone(), "test_task")?;

     // Create test task with ActionSpec
     let action_spec = ActionSpec::Python("print(f\"Hello {name}\")".to_string());

     let mut task_envs = HashMap::new();
     task_envs.insert("name".to_string(), "world".to_string());

     let mut task_values = HashMap::new();
     task_values.insert("name".to_string(), Value::Item("world".to_string()));

     let task = Task::new(
         "test_task".to_string(),
         vec!["dep_task".to_string()],
         vec![],
         vec![],
         task_envs,
         task_values,
         action_spec,
     );

     // Process the action
     let result = processor.process(&task.action, &task)?;

     // Verify the result
     match result {
         ProcessedAction::Python { path, script, hash } => {
             assert!(path.exists());
             assert!(script.contains("#!/usr/bin/env python3"));
             assert!(script.contains("OTTO_INPUT = {}"));
             assert!(script.contains("print(f\"Hello {name}\")"));
             // ... other assertions
         },
         _ => panic!("Expected Python variant"),
     }

     Ok(())
 }

#[test]
fn test_action_spec_validation() {
    // Test validation logic for ActionSpec
}

#[test]
fn test_conflicting_actions_error() {
    // Test YAML with both bash and python keys
}
```

**Add new validation tests:**
```rust
#[test]
fn test_empty_action_validation() {
    let yaml = r#"
tasks:
  test:
    bash: ""
"#;
    let result: Result<ConfigSpec, _> = serde_yaml::from_str(yaml);
    assert!(result.is_err());
}

#[test]
fn test_missing_action_validation() {
    let yaml = r#"
tasks:
  test:
    help: "test task"
"#;
    let result: Result<ConfigSpec, _> = serde_yaml::from_str(yaml);
    assert!(result.is_err());
}
```

#### 3.2 Integration Tests

**Update example files:**
- Convert all `examples/*/otto.yml` files to new format
- Ensure all tests pass with new structure

#### 3.3 Example Updates

**Convert existing examples:**
```yaml
# examples/ex1/otto.yml - BEFORE
tasks:
  punch:
    action: |
      #!/bin/bash
      echo "${arg:-donkey}"

# examples/ex1/otto.yml - AFTER
tasks:
  punch:
    bash: |
      echo "${arg:-donkey}"
```

### Phase 4: Documentation and Migration

#### 4.1 Documentation Updates

- Update README.md with new syntax
- Update any design documents
- Create migration guide
- Update API documentation

#### 4.2 Migration Guide

Create `docs/migration-action-keys.md`:
```markdown
# Migrating from action: to bash:/python:

## Quick Migration

### Before
```yaml
tasks:
  example:
    action: |
      #!/bin/bash
      echo "hello"
```

### After
```yaml
tasks:
  example:
    bash: |
      echo "hello"
```

## Migration Script
We provide a migration script to automatically convert your otto.yml files...
```

## Implementation Timeline

### Week 1: Core Structure
- [ ] Extract `deserialize_script_content` function from existing `deserialize_script`
- [ ] Implement ActionSpec enum and custom deserializer
- [ ] Update TaskSpec structure to use ActionSpec
- [ ] Update Task runtime structure
- [ ] Basic validation logic

### Week 2: Action Processing
- [ ] Update ActionProcessor to use ActionSpec
- [ ] Remove all shebang detection logic
- [ ] Update ScriptProcessor prologue generation (add shebangs)
- [ ] Update scheduler integration
- [ ] Remove Python3 → Python in ProcessedAction

### Week 3: Testing and Validation
- [ ] Update ALL unit tests in `src/executor/action.rs`
- [ ] Update ALL integration tests
- [ ] Add comprehensive validation tests
- [ ] Test error handling and edge cases
- [ ] Ensure NO compiler warnings
- [ ] Remove any `#[allow(dead_code)]` attributes
- [ ] Remove any unused variables (no underscored variables)

### Week 4: Examples and Documentation
- [ ] Convert ALL example files in `examples/` directory to new format
- [ ] Update documentation and README
- [ ] Create migration guide
- [ ] Final testing - ALL tests must pass
- [ ] Code cleanup and review

## Risk Mitigation

### Breaking Changes
- This is a breaking change that requires YAML file updates
- Consider providing a migration tool/script
- Provide clear error messages for old format

### Backward Compatibility
- During development, maintain both formats temporarily
- Provide deprecation warnings for old format
- Clear migration timeline

### Testing Strategy
- Comprehensive unit test coverage
- Integration tests with real YAML files
- Test error conditions and edge cases
- Performance testing to ensure no regression

## Future Extensibility

This structure makes it easy to add new languages:
```yaml
tasks:
  example:
    nodejs: |
      console.log("Hello World");
    # or
    ruby: |
      puts "Hello World"
    # or
    go: |
      fmt.Println("Hello World")
```

Each would just need a new processor implementing the `ScriptProcessor` trait.

## Success Criteria

- [ ] All existing functionality works with new format
- [ ] No performance regression
- [ ] Clear error messages for validation failures
- [ ] **ALL tests pass with NO warnings**
- [ ] **NO `#[allow(dead_code)]` attributes**
- [ ] **NO unused variables (no underscored variables)**
- [ ] **ALL example files converted to new format**
- [ ] ProcessedAction uses `Python` not `Python3`
- [ ] Reusable `deserialize_script_content` function implemented
- [ ] Documentation is updated
- [ ] Migration path is clear and well-documented
- [ ] Code is cleaner and more maintainable

## Critical Implementation Notes

### Function Reuse
- Extract existing `deserialize_script` logic into `deserialize_script_content` function
- Call this function from both the ActionSpec deserializer and any other places that need script content processing

### ProcessedAction Consistency
- Change `Python3` to `Python` in ProcessedAction enum to match ActionSpec
- Update all pattern matching accordingly

### Code Quality Requirements
- Zero compiler warnings
- No dead code attributes
- No underscored unused variables
- All tests passing

### Complete Migration
- Every single example file must be converted
- Every single test must be updated
- No legacy format support in final implementation

## Files That Must Be Updated

### Core Implementation Files
- `src/cfg/task.rs` - TaskSpec structure and ActionSpec enum
- `src/executor/task.rs` - Task runtime structure
- `src/executor/action.rs` - ActionProcessor and ProcessedAction
- `src/executor/scheduler.rs` - Task execution logic
- `src/cli/parser.rs` - Task creation calls

### All Test Files
- `src/executor/action.rs` - Unit tests (update ALL test functions)
- Any integration test files that create tasks
- Any test files that parse YAML with actions

### All Example Files (Convert action: to bash:/python:)
- `examples/ex1/otto.yml`
- `examples/ex2/.otto.yml`
- `examples/ex3/otto.yaml`
- `examples/ex4/otto.yml` (if it has actions)
- `examples/ex5/otto.yml`
- `examples/ex6/otto.yml`
- `examples/ex7/otto.yml`
- `examples/ex8/otto.yml`
- `examples/ex9/otto.yml`
- `examples/ex10/otto.yml`
- `examples/ex11/otto.yml`
- `examples/ex12/otto.yml`
- `examples/ex13/otto.yml`
- `examples/auth-svc/otto.yml`
- `examples/devs/otto.yml`
- `examples/flags/flag_demo.yml`
- `examples/media-planning-service/otto.yml`
- `examples/pre-commit-hooks/otto.yml`
- `examples/old/ex1/otto.yml` (if still relevant)
- Any other YAML files in examples/

### Documentation Files
- `README.md` - Update syntax examples
- Any other docs with YAML examples