use eyre::Result;

use super::ast::{AssignmentType, MakefileAst, Target, Variable};

pub struct MakefileParser {
    content: String,
}

impl MakefileParser {
    pub fn new(content: String) -> Self {
        Self { content }
    }

    pub fn parse(&mut self) -> Result<MakefileAst> {
        let mut ast = MakefileAst::new();
        let lines: Vec<String> = self.content.lines().map(|s| s.to_string()).collect();
        let mut i = 0;
        let mut last_comment: Option<String> = None;

        while i < lines.len() {
            let line = &lines[i];
            let trimmed = line.trim();

            // Skip empty lines
            if trimmed.is_empty() {
                last_comment = None;
                i += 1;
                continue;
            }

            // Handle comments
            if let Some(comment_text) = trimmed.strip_prefix('#') {
                last_comment = Some(comment_text.trim().to_string());
                i += 1;
                continue;
            }

            // Check for .PHONY declaration
            if let Some(phony_targets) = self.is_phony_declaration(trimmed) {
                for target in phony_targets {
                    ast.phony_targets.insert(target);
                }
                last_comment = None;
                i += 1;
                continue;
            }

            // Check for .DEFAULT_GOAL
            if let Some(goal) = self.extract_default_goal(trimmed) {
                ast.default_goal = Some(goal);
                last_comment = None;
                i += 1;
                continue;
            }

            // Check for variable assignment
            if let Some(var) = self.parse_variable(line)? {
                ast.variables.push(var);
                last_comment = None;
                i += 1;
                continue;
            }

            // Check for target definition
            if trimmed.contains(':')
                && !trimmed.starts_with('\t')
                && let Some(target) = self.parse_target(&lines, &mut i, last_comment.clone())?
            {
                ast.targets.push(target);
                last_comment = None;
                continue;
            }

            // Unknown line, skip it
            last_comment = None;
            i += 1;
        }

        Ok(ast)
    }

    fn parse_variable(&self, line: &str) -> Result<Option<Variable>> {
        // Check for different assignment operators
        let assignment_ops = [":=", "?=", "+=", "="];

        for op in &assignment_ops {
            if let Some(pos) = line.find(op) {
                // Make sure it's not inside a recipe (tab-indented)
                if line.starts_with('\t') {
                    return Ok(None);
                }

                let name = line[..pos].trim().to_string();
                let value_start = pos + op.len();
                let value = if value_start < line.len() {
                    line[value_start..].trim().to_string()
                } else {
                    String::new()
                };

                // Skip lines that look like targets (e.g., "target := dependency")
                if name.is_empty() || name.contains(':') && *op != ":=" {
                    return Ok(None);
                }

                let assignment_type = match *op {
                    ":=" => {
                        // Check if it's a shell command
                        if value.contains("$(shell ") || value.starts_with("$(shell ") {
                            AssignmentType::ShellExecution
                        } else {
                            AssignmentType::Simple
                        }
                    }
                    "?=" => AssignmentType::Conditional,
                    "+=" => AssignmentType::Append,
                    "=" => AssignmentType::Recursive,
                    _ => AssignmentType::Recursive,
                };

                return Ok(Some(Variable {
                    name,
                    value,
                    assignment_type,
                }));
            }
        }

        Ok(None)
    }

    fn parse_target(&self, lines: &[String], index: &mut usize, comment: Option<String>) -> Result<Option<Target>> {
        let line = &lines[*index];
        let trimmed = line.trim();

        // Skip if line starts with tab (it's a command, not a target)
        if line.starts_with('\t') {
            return Ok(None);
        }

        // Find the colon that separates target from dependencies
        let colon_pos = match trimmed.find(':') {
            Some(pos) => pos,
            None => return Ok(None),
        };

        // Extract target name
        let target_name = trimmed[..colon_pos].trim().to_string();

        // Skip special targets that aren't real targets
        if target_name.is_empty() || target_name.starts_with('.') && target_name != ".PHONY" {
            *index += 1;
            return Ok(None);
        }

        // Extract dependencies
        let dep_part = if colon_pos + 1 < trimmed.len() { trimmed[colon_pos + 1..].trim() } else { "" };

        let dependencies = self.parse_dependencies(dep_part);

        // Move to next line to parse commands
        *index += 1;

        // Parse commands (tab-indented lines following the target)
        let commands = self.parse_commands(lines, index);

        Ok(Some(Target {
            name: target_name.clone(),
            dependencies,
            commands,
            comment,
            is_phony: false, // Will be set later based on .PHONY declarations
        }))
    }

    fn parse_dependencies(&self, dep_line: &str) -> Vec<String> {
        dep_line.split_whitespace().map(|s| s.to_string()).collect()
    }

    fn parse_commands(&self, lines: &[String], index: &mut usize) -> Vec<String> {
        let mut commands = Vec::new();

        while *index < lines.len() {
            let line = &lines[*index];

            // Commands must start with a tab
            if !line.starts_with('\t') {
                break;
            }

            // Remove the leading tab and add to commands
            let command = line[1..].to_string();

            // Handle line continuations
            if command.trim_end().ends_with('\\') {
                let mut continued_command = command.trim_end().trim_end_matches('\\').to_string();
                *index += 1;

                while *index < lines.len() {
                    let next_line = &lines[*index];
                    if let Some(next_cmd_str) = next_line.strip_prefix('\t') {
                        let next_cmd = next_cmd_str.to_string();
                        let trimmed_next = next_cmd.trim_start();
                        continued_command.push(' ');
                        continued_command.push_str(trimmed_next);

                        if !next_cmd.trim_end().ends_with('\\') {
                            *index += 1;
                            break;
                        }
                        continued_command = continued_command.trim_end().trim_end_matches('\\').to_string();
                        *index += 1;
                    } else {
                        break;
                    }
                }

                commands.push(continued_command);
            } else {
                commands.push(command);
                *index += 1;
            }
        }

        commands
    }

    fn is_phony_declaration(&self, line: &str) -> Option<Vec<String>> {
        line.strip_prefix(".PHONY:")
            .map(|targets_part| targets_part.split_whitespace().map(|s| s.to_string()).collect())
    }

    fn extract_default_goal(&self, line: &str) -> Option<String> {
        if line.starts_with(".DEFAULT_GOAL") {
            if let Some(pos) = line.find(":=") {
                let goal = line[pos + 2..].trim();
                return Some(goal.to_string());
            } else if let Some(pos) = line.find('=') {
                let goal = line[pos + 1..].trim();
                return Some(goal.to_string());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_variable() {
        let content = "VAR := value".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.variables.len(), 1);
        assert_eq!(ast.variables[0].name, "VAR");
        assert_eq!(ast.variables[0].value, "value");
        assert_eq!(ast.variables[0].assignment_type, AssignmentType::Simple);
    }

    #[test]
    fn test_parse_recursive_variable() {
        let content = "VAR = value".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.variables.len(), 1);
        assert_eq!(ast.variables[0].assignment_type, AssignmentType::Recursive);
    }

    #[test]
    fn test_parse_conditional_variable() {
        let content = "VAR ?= default".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.variables.len(), 1);
        assert_eq!(ast.variables[0].assignment_type, AssignmentType::Conditional);
    }

    #[test]
    fn test_parse_shell_variable() {
        let content = "VERSION := $(shell git describe --tags)".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.variables.len(), 1);
        assert_eq!(ast.variables[0].assignment_type, AssignmentType::ShellExecution);
        assert!(ast.variables[0].value.contains("$(shell"));
    }

    #[test]
    fn test_parse_simple_target() {
        let content = "build:\n\techo Building".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.targets.len(), 1);
        assert_eq!(ast.targets[0].name, "build");
        assert_eq!(ast.targets[0].commands.len(), 1);
        assert_eq!(ast.targets[0].commands[0], "echo Building");
    }

    #[test]
    fn test_parse_target_with_dependencies() {
        let content = "build: test clean\n\techo Building".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.targets.len(), 1);
        assert_eq!(ast.targets[0].dependencies.len(), 2);
        assert_eq!(ast.targets[0].dependencies[0], "test");
        assert_eq!(ast.targets[0].dependencies[1], "clean");
    }

    #[test]
    fn test_parse_target_with_comment() {
        let content = "# Build the project\nbuild:\n\techo Building".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.targets.len(), 1);
        assert_eq!(ast.targets[0].comment, Some("Build the project".to_string()));
    }

    #[test]
    fn test_parse_phony_declaration() {
        let content = ".PHONY: build clean test".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.phony_targets.len(), 3);
        assert!(ast.phony_targets.contains("build"));
        assert!(ast.phony_targets.contains("clean"));
        assert!(ast.phony_targets.contains("test"));
    }

    #[test]
    fn test_parse_default_goal() {
        let content = ".DEFAULT_GOAL := build".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.default_goal, Some("build".to_string()));
    }

    #[test]
    fn test_parse_multiline_command() {
        let content = "build:\n\tmkdir -p dist && \\\n\techo Done".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.targets.len(), 1);
        assert_eq!(ast.targets[0].commands.len(), 1);
        assert!(ast.targets[0].commands[0].contains("mkdir -p dist"));
        assert!(ast.targets[0].commands[0].contains("echo Done"));
    }

    #[test]
    fn test_parse_multiple_targets() {
        let content = "build:\n\techo Building\n\ntest:\n\techo Testing".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.targets.len(), 2);
        assert_eq!(ast.targets[0].name, "build");
        assert_eq!(ast.targets[1].name, "test");
    }

    #[test]
    fn test_parse_target_with_multiple_commands() {
        let content = "build:\n\techo Starting\n\tmkdir -p dist\n\techo Done".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.targets.len(), 1);
        assert_eq!(ast.targets[0].commands.len(), 3);
    }

    #[test]
    fn test_parse_empty_makefile() {
        let content = "".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.variables.len(), 0);
        assert_eq!(ast.targets.len(), 0);
    }

    #[test]
    fn test_parse_comments_only() {
        let content = "# This is a comment\n# Another comment".to_string();
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.variables.len(), 0);
        assert_eq!(ast.targets.len(), 0);
    }

    #[test]
    fn test_parse_complex_makefile() {
        let content = r#".DEFAULT_GOAL := build

VAR1 := value1
VAR2 = value2
VERSION := $(shell git describe --tags)

.PHONY: build clean test

# Build the project
build: test
	echo "Building version $(VERSION)"
	mkdir -p dist

# Run tests
test:
	go test ./...

clean:
	rm -rf dist
"#
        .to_string();

        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().unwrap();

        assert_eq!(ast.default_goal, Some("build".to_string()));
        assert_eq!(ast.variables.len(), 3);
        assert_eq!(ast.phony_targets.len(), 3);
        assert_eq!(ast.targets.len(), 3);
    }
}
