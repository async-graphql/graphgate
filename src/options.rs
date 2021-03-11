use structopt::StructOpt;

#[derive(StructOpt)]
pub struct Options {
    /// Path of the config file
    #[structopt(default_value = "config.toml")]
    pub config: String,
}
