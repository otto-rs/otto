pub mod cfg;
pub mod executor;
pub mod cli;

pub use cfg::otto::OttoSpec;
pub use cli::NomParser as Parser;

pub use executor::{Task, TaskScheduler, TaskStatus, Workspace};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
