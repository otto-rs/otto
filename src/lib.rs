pub mod cli;
pub mod cfg;
pub mod executor;

// Bespoke CLI implementation (previously called nom-cli)
#[cfg(feature = "bespoke-cli")]
pub mod cli2;

// True nom-based CLI implementation
#[cfg(feature = "nom-cli")]
pub mod cli3;

pub use cfg::otto::OttoSpec;

// Export the appropriate CLI parser based on features
#[cfg(all(feature = "clap-cli", not(feature = "bespoke-cli"), not(feature = "nom-cli")))]
pub use cli::parse::Parser;

#[cfg(feature = "bespoke-cli")]
pub use cli2::BespokeParser as Parser;

#[cfg(feature = "nom-cli")]
pub use cli3::NomParser as Parser;

pub use executor::{Task, TaskScheduler, TaskStatus, TaskType, Workspace};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
