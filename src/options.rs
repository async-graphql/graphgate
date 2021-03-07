use structopt::StructOpt;

#[derive(StructOpt)]
pub enum Options {
    /// Start the gateway using the configuration file
    Serve {
        /// Path of the config file
        config: String,
    },

    /// Start the gateway using the schema definition file
    Schema {
        /// Path of the schema file
        schema: String,

        /// Bind address
        #[structopt(long)]
        bind: String,
    },

    /// Start gateway in kubernetes
    K8s {
        /// Bind address
        #[structopt(long)]
        bind: String,
    },
}
