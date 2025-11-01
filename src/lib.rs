pub mod cfg;
pub mod cli;
pub mod executor;
pub mod utils;

pub use cfg::config::ConfigSpec;
pub use cli::Parser;
pub use executor::{Task, TaskScheduler, Workspace};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
