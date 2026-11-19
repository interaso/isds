use std::error::Error;
use std::{env, io};

use isds::{Client, Credentials, Environment};

/// Builds a client from environment variables without embedding credentials.
pub fn client() -> Result<Client, Box<dyn Error>> {
    let username = required_env("ISDS_EXAMPLE_USERNAME")?;
    let password = required_env("ISDS_EXAMPLE_PASSWORD")?;
    let environment = match env::var("ISDS_EXAMPLE_ENVIRONMENT") {
        Ok(value) if value == "test" => Environment::Test,
        Ok(value) if value == "production" => Environment::Production,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ISDS_EXAMPLE_ENVIRONMENT must be test or production",
            )
            .into());
        }
        Err(env::VarError::NotPresent) => Environment::Test,
        Err(env::VarError::NotUnicode(_)) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ISDS_EXAMPLE_ENVIRONMENT is not Unicode",
            )
            .into());
        }
    };

    Ok(Client::builder(environment)
        .credentials(Credentials::new(username, password))
        .build()?)
}

fn required_env(name: &str) -> Result<String, io::Error> {
    env::var(name).map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("set {name} in the environment"),
        )
    })
}
