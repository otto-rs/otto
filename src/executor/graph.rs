use eyre::{Result, eyre};
use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::process::Command;

use super::task::{DAG, Task};
use crate::cli::Parser;

/// Graph visualization options
#[derive(Debug, Clone)]
pub struct GraphOptions {
    /// Include task details in nodes
    pub show_details: bool,
    /// Show file dependencies
    pub show_file_deps: bool,
    /// Output format preference
    pub format: GraphFormat,
    /// Node styling
    pub style: NodeStyle,
    /// Output file path (optional)
    pub output_path: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone)]
pub enum GraphFormat {
    /// Generate SVG image (requires graphviz)
    Svg,
    /// Generate PNG image (requires graphviz)
    Png,
    /// Generate PDF (requires graphviz)
    Pdf,
    /// Raw DOT format
    Dot,
    /// ASCII art for terminal
    Ascii,
    /// Auto-detect based on file extension
    Auto,
}

#[derive(Debug, Clone)]
pub enum NodeStyle {
    Simple,
    Detailed,
    Compact,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self {
            show_details: true,
            show_file_deps: true,
            format: GraphFormat::Svg,
            style: NodeStyle::Detailed,
            output_path: None,
        }
    }
}

/// DAG visualizer for Otto tasks
pub struct DagVisualizer {
    options: GraphOptions,
}

impl DagVisualizer {
    pub fn new(options: GraphOptions) -> Self {
        Self { options }
    }

    pub fn with_defaults() -> Self {
        Self::new(GraphOptions::default())
    }

    pub async fn execute_command(task: &crate::cli::parser::Task) -> Result<()> {
        // Parse graph command arguments
        let format = task
            .values
            .get("format")
            .and_then(|v| match v {
                crate::cfg::config::Value::Item(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("ascii");

        let output_path = task.values.get("output").and_then(|v| match v {
            crate::cfg::config::Value::Item(s) => Some(std::path::PathBuf::from(s)),
            _ => None,
        });

        let graph_format = match format {
            "ascii" => GraphFormat::Ascii,
            "dot" => GraphFormat::Dot,
            "svg" => GraphFormat::Svg,
            "png" => GraphFormat::Png,
            "pdf" => GraphFormat::Pdf,
            _ => GraphFormat::Ascii,
        };

        let options = GraphOptions {
            show_details: true,
            show_file_deps: true,
            format: graph_format,
            style: NodeStyle::Detailed,
            output_path,
        };

        // We need to reload the parser to get all tasks for the graph
        let args: Vec<String> = env::args().collect();
        let mut parser = Parser::new(args)?;
        let (all_tasks, _, _) = parser.parse_all_tasks()?;

        let dag = Self::from_tasks(all_tasks)?;

        let visualizer = DagVisualizer::new(options);
        let result = visualizer.visualize(&dag)?;

        println!("{result}");

        Ok(())
    }

    pub fn from_tasks(tasks: Vec<crate::cli::parser::Task>) -> Result<DAG<Task>> {
        // Convert parser tasks to executor tasks
        let executor_tasks: Vec<Task> = tasks
            .into_iter()
            .filter(|task| task.name != "graph") // Exclude graph task itself
            .map(|parser_task| {
                Task::new(
                    parser_task.name,
                    parser_task.task_deps,
                    parser_task.file_deps,
                    parser_task.output_deps,
                    parser_task.envs,
                    parser_task.values,
                    parser_task.action,
                    parser_task.interactive,
                )
            })
            .collect();

        Self::create_dag_from_tasks(executor_tasks)
    }

    fn create_dag_from_tasks(mut tasks: Vec<Task>) -> Result<DAG<Task>> {
        use daggy::Dag;

        let mut dag: DAG<Task> = Dag::new();
        let mut task_indices = HashMap::new();

        // Sort tasks alphabetically for consistent ordering
        tasks.sort_by(|a, b| a.name.cmp(&b.name));

        for task in tasks {
            let index = dag.add_node(task.clone());
            task_indices.insert(task.name.clone(), index);
        }

        let mut edges_to_add = Vec::new();
        for (node_index, node_data) in dag.raw_nodes().iter().enumerate() {
            let task = &node_data.weight;
            let current_index = daggy::NodeIndex::new(node_index);

            for dep_name in &task.task_deps {
                if let Some(&dep_index) = task_indices.get(dep_name) {
                    edges_to_add.push((dep_index, current_index, task.name.clone()));
                }
            }
        }

        for (dep_index, current_index, task_name) in edges_to_add {
            dag.add_edge(dep_index, current_index, ())
                .map_err(|e| eyre!("Failed to add edge to {}: {:?}", task_name, e))?;
        }

        Ok(dag)
    }

    /// Visualize the DAG and save to file or display
    pub fn visualize(&self, dag: &DAG<Task>) -> Result<String> {
        match self.options.format {
            GraphFormat::Ascii => self.generate_ascii(dag),
            GraphFormat::Dot => self.generate_dot(dag),
            GraphFormat::Svg | GraphFormat::Png | GraphFormat::Pdf | GraphFormat::Auto => self.generate_image(dag),
        }
    }

    pub fn generate_image(&self, dag: &DAG<Task>) -> Result<String> {
        let dot_content = self.generate_dot(dag)?;

        // Determine output format and path
        let (format, output_path) = self.determine_output_format()?;

        if !self.is_graphviz_available() {
            return Err(eyre!(
                "Graphviz not found. Please install graphviz to generate images.\n\
                On Ubuntu/Debian: sudo apt install graphviz\n\
                On macOS: brew install graphviz\n\
                On Windows: Download from https://graphviz.org/download/\n\
                \n\
                Falling back to ASCII output:\n{}",
                self.generate_ascii(dag)?
            ));
        }

        let temp_dir = tempfile::tempdir()?;
        let dot_file = temp_dir.path().join("otto_graph.dot");
        std::fs::write(&dot_file, &dot_content)?;

        // Run graphviz to generate image
        let output = Command::new("dot")
            .arg(format!("-T{format}"))
            .arg(&dot_file)
            .arg("-o")
            .arg(&output_path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(eyre!("Graphviz failed: {}", stderr));
        }

        Ok(format!(
            "Graph visualization saved to: {}\n\
            Format: {}\n\
            Open with your preferred image viewer or browser.",
            output_path.display(),
            format.to_uppercase()
        ))
    }

    pub fn generate_dot(&self, dag: &DAG<Task>) -> Result<String> {
        let mut dot = String::new();

        // Start digraph
        dot.push_str("digraph otto_dag {\n");
        dot.push_str("  label=\"Otto Task DAG\";\n");
        dot.push_str("  labelloc=\"t\";\n");
        dot.push_str("  fontsize=\"16\";\n");
        dot.push_str("  fontname=\"Helvetica\";\n");
        dot.push_str("  rankdir=\"TB\";\n");
        dot.push_str("  bgcolor=\"white\";\n");
        dot.push_str("  \n");

        // Default node attributes
        dot.push_str("  node [\n");
        dot.push_str("    shape=\"box\",\n");
        dot.push_str("    style=\"rounded,filled\",\n");
        dot.push_str("    fontname=\"Helvetica\",\n");
        dot.push_str("    fontsize=\"12\"\n");
        dot.push_str("  ];\n");
        dot.push_str("  \n");

        // Default edge attributes
        dot.push_str("  edge [\n");
        dot.push_str("    fontname=\"Helvetica\",\n");
        dot.push_str("    fontsize=\"10\"\n");
        dot.push_str("  ];\n");
        dot.push_str("  \n");

        let mut task_to_id = HashMap::new();
        let mut file_nodes = std::collections::HashSet::new();

        for (idx, node) in dag.raw_nodes().iter().enumerate() {
            let task = &node.weight;
            let node_id = format!("task_{idx}");
            task_to_id.insert(task.name.clone(), node_id.clone());

            let label = self.create_node_label(task);
            let escaped_label = self.escape_dot_string(&label);

            // Determine node color based on task characteristics
            let color = if !task.file_deps.is_empty() || !task.output_deps.is_empty() {
                "lightblue"
            } else {
                "lightgray"
            };

            dot.push_str(&format!(
                "  {node_id} [label=\"{escaped_label}\", fillcolor=\"{color}\"];\n"
            ));
        }

        dot.push_str("  \n");

        for node in dag.raw_nodes() {
            let task = &node.weight;
            if let Some(target_id) = task_to_id.get(&task.name) {
                for dep_name in &task.task_deps {
                    if let Some(source_id) = task_to_id.get(dep_name) {
                        dot.push_str(&format!(
                            "  {source_id} -> {target_id} [label=\"depends\", color=\"black\"];\n"
                        ));
                    }
                }
            }
        }

        if self.options.show_file_deps {
            self.add_file_dependencies_to_dot(&mut dot, dag, &task_to_id, &mut file_nodes)?;
        }

        dot.push_str("}\n");

        Ok(dot)
    }

    pub fn generate_ascii(&self, dag: &DAG<Task>) -> Result<String> {
        let mut output = String::new();

        output.push_str("┌─────────────────────────────────────┐\n");
        output.push_str("│           Otto Task DAG             │\n");
        output.push_str("└─────────────────────────────────────┘\n\n");

        // Find leaf tasks (tasks that nothing depends on - the top-level tasks you'd run)
        let all_task_names: std::collections::HashSet<String> =
            dag.raw_nodes().iter().map(|node| node.weight.name.clone()).collect();

        let mut leaf_tasks: Vec<_> = dag
            .raw_nodes()
            .iter()
            .filter(|node| {
                // A task is a leaf if no other task depends on it
                !all_task_names.iter().any(|other_task_name| {
                    dag.raw_nodes().iter().any(|other_node| {
                        other_node.weight.name == *other_task_name
                            && other_node.weight.task_deps.contains(&node.weight.name)
                    })
                })
            })
            .collect();

        // Sort leaf tasks alphabetically for consistent output
        leaf_tasks.sort_by(|a, b| a.weight.name.cmp(&b.weight.name));

        if leaf_tasks.is_empty() {
            output.push_str("No leaf tasks found (possible circular dependencies)\n");
            return Ok(output);
        }

        for (i, leaf) in leaf_tasks.iter().enumerate() {
            let is_last_leaf = i == leaf_tasks.len() - 1;
            Self::render_ascii_subtree(
                &mut output,
                &leaf.weight,
                dag,
                0,
                &mut std::collections::HashSet::new(),
                is_last_leaf,
            )?;
        }

        output.push_str("\n┌─────────────────────────────────────┐\n");
        output.push_str("│ Legend:                             │\n");
        output.push_str("│ ├─ Task name [inputs:N] [outputs:M] │\n");
        output.push_str("│ └─ Dependencies flow top to bottom  │\n");
        output.push_str("└─────────────────────────────────────┘\n");

        Ok(output)
    }

    fn create_node_label(&self, task: &Task) -> String {
        match self.options.style {
            NodeStyle::Simple => task.name.clone(),
            NodeStyle::Compact => {
                format!("{}\n[{}]", task.name, &task.hash[..6])
            }
            NodeStyle::Detailed => {
                let mut label = task.name.clone();

                if self.options.show_details {
                    if !task.file_deps.is_empty() {
                        label.push_str(&format!("\nInputs: {}", task.file_deps.len()));
                    }
                    if !task.output_deps.is_empty() {
                        label.push_str(&format!("\nOutputs: {}", task.output_deps.len()));
                    }
                    if !task.envs.is_empty() {
                        label.push_str(&format!("\nEnvs: {}", task.envs.len()));
                    }
                }

                label.push_str(&format!("\n[{}]", &task.hash[..6]));
                label
            }
        }
    }

    /// Escape strings for DOT format
    fn escape_dot_string(&self, s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
            .replace('$', "\\$") // Escape dollar signs to prevent variable expansion
            .replace('{', "\\{") // Escape braces
            .replace('}', "\\}")
    }

    fn add_file_dependencies_to_dot(
        &self,
        dot: &mut String,
        dag: &DAG<Task>,
        task_to_id: &HashMap<String, String>,
        file_nodes: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        dot.push_str("  \n  // File dependencies\n");

        for node in dag.raw_nodes() {
            let task = &node.weight;
            if let Some(task_id) = task_to_id.get(&task.name) {
                for file_dep in &task.file_deps {
                    let file_id = format!("file_{}", file_dep.replace(['/', '.', '*', '-', '$', '{', '}'], "_"));

                    if !file_nodes.contains(&file_id) {
                        let escaped_label = self.escape_dot_string(file_dep);
                        dot.push_str(&format!(
                            "  {file_id} [label=\"{escaped_label}\", shape=\"ellipse\", fillcolor=\"lightgreen\"];\n"
                        ));
                        file_nodes.insert(file_id.clone());
                    }

                    dot.push_str(&format!(
                        "  {file_id} -> {task_id} [label=\"input\", color=\"green\", style=\"dashed\"];\n"
                    ));
                }

                for output_dep in &task.output_deps {
                    let file_id = format!(
                        "output_{}",
                        output_dep.replace(['/', '.', '*', '-', '$', '{', '}'], "_")
                    );

                    if !file_nodes.contains(&file_id) {
                        let escaped_label = self.escape_dot_string(output_dep);
                        dot.push_str(&format!(
                            "  {file_id} [label=\"{escaped_label}\", shape=\"ellipse\", fillcolor=\"lightyellow\"];\n"
                        ));
                        file_nodes.insert(file_id.clone());
                    }

                    dot.push_str(&format!(
                        "  {task_id} -> {file_id} [label=\"output\", color=\"orange\", style=\"dashed\"];\n"
                    ));
                }
            }
        }

        Ok(())
    }

    fn render_ascii_subtree(
        output: &mut String,
        task: &Task,
        dag: &DAG<Task>,
        depth: usize,
        visited: &mut std::collections::HashSet<String>,
        is_last: bool,
    ) -> Result<()> {
        let indent = "  ".repeat(depth);
        let connector = if is_last { "└─" } else { "├─" };

        if visited.contains(&task.name) {
            output.push_str(&format!("{}{} {} (circular ref)\n", indent, connector, task.name));
            return Ok(());
        }

        visited.insert(task.name.clone());

        // Show task info
        output.push_str(&format!("{}{} {}", indent, connector, task.name));
        if !task.file_deps.is_empty() {
            output.push_str(&format!(" [inputs:{}]", task.file_deps.len()));
        }
        if !task.output_deps.is_empty() {
            output.push_str(&format!(" [outputs:{}]", task.output_deps.len()));
        }
        output.push('\n');

        // Find tasks that this task depends on
        let dependencies: Vec<_> = task
            .task_deps
            .iter()
            .filter_map(|dep_name| dag.raw_nodes().iter().find(|node| node.weight.name == *dep_name))
            .collect();

        for (i, dependency) in dependencies.iter().enumerate() {
            let is_last_dependency = i == dependencies.len() - 1;
            Self::render_ascii_subtree(output, &dependency.weight, dag, depth + 1, visited, is_last_dependency)?;
        }

        visited.remove(&task.name);
        Ok(())
    }

    fn is_graphviz_available(&self) -> bool {
        Command::new("dot")
            .arg("-V")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn determine_output_format(&self) -> Result<(String, std::path::PathBuf)> {
        let (format, extension) = match &self.options.format {
            GraphFormat::Svg => ("svg", "svg"),
            GraphFormat::Png => ("png", "png"),
            GraphFormat::Pdf => ("pdf", "pdf"),
            GraphFormat::Auto => {
                if let Some(ref path) = self.options.output_path {
                    match path.extension().and_then(|s| s.to_str()) {
                        Some("svg") => ("svg", "svg"),
                        Some("png") => ("png", "png"),
                        Some("pdf") => ("pdf", "pdf"),
                        _ => ("svg", "svg"), // default
                    }
                } else {
                    ("svg", "svg") // default
                }
            }
            _ => return Err(eyre!("Invalid format for image generation")),
        };

        let output_path = if let Some(ref path) = self.options.output_path {
            path.clone()
        } else {
            std::env::current_dir()?.join(format!("otto_graph.{extension}"))
        };

        Ok((format.to_string(), output_path))
    }

    pub fn write_dot_file(&self, dag: &DAG<Task>, path: &Path) -> Result<()> {
        let dot_content = self.generate_dot(dag)?;
        std::fs::write(path, dot_content)?;
        Ok(())
    }

    pub fn write_ascii_file(&self, dag: &DAG<Task>, path: &Path) -> Result<()> {
        let ascii_content = self.generate_ascii(dag)?;
        std::fs::write(path, ascii_content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_task(name: &str, deps: Vec<&str>) -> Task {
        Task::new(
            name.to_string(),
            deps.into_iter().map(String::from).collect(),
            vec![],
            vec![],
            HashMap::new(),
            HashMap::new(),
            format!("echo 'Running {name}'"),
            false,
        )
    }

    #[test]
    fn test_dot_generation_simple() -> Result<()> {
        let mut dag = DAG::new();

        let task1 = create_test_task("build", vec![]);
        let task2 = create_test_task("test", vec!["build"]);

        dag.add_node(task1);
        dag.add_node(task2);

        let visualizer = DagVisualizer::with_defaults();
        let dot = visualizer.generate_dot(&dag)?;

        assert!(dot.contains("digraph otto_dag"));
        assert!(dot.contains("task_0"));
        assert!(dot.contains("task_1"));
        assert!(dot.contains("Otto Task DAG"));

        Ok(())
    }

    #[test]
    fn test_ascii_generation() -> Result<()> {
        let mut dag = DAG::new();

        let task1 = create_test_task("setup", vec![]);
        let task2 = create_test_task("build", vec!["setup"]);
        let task3 = create_test_task("test", vec!["build"]);

        dag.add_node(task1);
        dag.add_node(task2);
        dag.add_node(task3);

        let visualizer = DagVisualizer::with_defaults();
        let ascii = visualizer.generate_ascii(&dag)?;

        assert!(ascii.contains("Otto Task DAG"));
        assert!(ascii.contains("setup"));
        assert!(ascii.contains("build"));
        assert!(ascii.contains("test"));
        assert!(ascii.contains("Legend"));

        Ok(())
    }

    #[test]
    fn test_graphviz_detection() {
        let visualizer = DagVisualizer::with_defaults();
        // This test will pass regardless of whether graphviz is installed
        let _has_graphviz = visualizer.is_graphviz_available();
    }

    #[test]
    fn test_dot_string_escaping() {
        let visualizer = DagVisualizer::with_defaults();
        assert_eq!(visualizer.escape_dot_string("hello"), "hello");
        assert_eq!(visualizer.escape_dot_string("hello\nworld"), "hello\\nworld");
        assert_eq!(visualizer.escape_dot_string("say \"hello\""), "say \\\"hello\\\"");
        assert_eq!(visualizer.escape_dot_string("path\\to\\file"), "path\\\\to\\\\file");
    }
}
