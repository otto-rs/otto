pub mod cli;
pub mod cfg;
pub mod executor;

pub use cfg::otto::OttoSpec;

// Export the nom-based CLI parser
pub use cli::NomParser as Parser;

pub use executor::{Task, TaskScheduler, TaskStatus, TaskType, Workspace};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
