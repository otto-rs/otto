pub mod cli;
pub mod cfg;
pub mod executor;

pub use cfg::otto::Otto;
pub use cli::parse::Parser;
pub use executor::{TaskSpec, TaskScheduler, TaskStatus, TaskType, Workspace};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
