use clap::Parser;
use eyre::{Context, Result};
use std::io::{self, Read, Write};
use std::path::PathBuf;

use crate::makefile::{MakefileParser, OttoConverter};

/// Convert Makefile to Otto YAML format
#[derive(Parser, Debug)]
#[command(name = "convert")]
#[command(about = "Convert Makefile to Otto YAML format")]
pub struct ConvertCommand {
    /// Treat warnings as errors
    #[arg(long)]
    pub strict: bool,

    /// Output file (default: stdout)
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

impl ConvertCommand {
    pub fn execute(&self) -> Result<()> {
        // Read from stdin
        let mut content = String::new();
        io::stdin()
            .read_to_string(&mut content)
            .wrap_err("Failed to read from stdin")?;

        // Parse Makefile
        let mut parser = MakefileParser::new(content);
        let ast = parser.parse().wrap_err("Failed to parse Makefile")?;

        // Convert to Otto
        let converter = OttoConverter::new(ast);
        let config = converter.convert().wrap_err("Failed to convert to Otto format")?;

        // Serialize to YAML
        let yaml = serde_yaml::to_string(&config).wrap_err("Failed to serialize to YAML")?;

        // Write to stdout or file
        if let Some(output_path) = &self.output {
            std::fs::write(output_path, yaml)
                .wrap_err_with(|| format!("Failed to write to file: {}", output_path.display()))?;
        } else {
            io::stdout()
                .write_all(yaml.as_bytes())
                .wrap_err("Failed to write to stdout")?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_command_creation() {
        let cmd = ConvertCommand {
            strict: false,
            output: None,
        };
        assert!(!cmd.strict);
        assert!(cmd.output.is_none());
    }

    #[test]
    fn test_convert_command_with_output() {
        let cmd = ConvertCommand {
            strict: true,
            output: Some(PathBuf::from("output.yml")),
        };
        assert!(cmd.strict);
        assert!(cmd.output.is_some());
    }
}

