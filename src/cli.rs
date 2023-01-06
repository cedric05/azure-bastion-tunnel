use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub(crate) struct Cli {
    /// if not provided, uses default subscription
    #[arg(short, long)]
    pub(crate) subscription_id: Option<String>,

    /// resource_group name
    #[arg(long)]
    pub(crate) resource_group: String,

    /// virtual machine name
    #[arg(short, long)]
    pub(crate) vm: String,

    /// bastion name
    #[arg(short, long)]
    pub(crate) bastion: String,

    #[arg(short, long, default_value = "tcptunnel")]
    pub(crate) tunnel: String,

    #[arg(long, default_value_t = 22)]
    pub(crate) remote_port: u16,

    #[arg(short, long, default_value_t = 2222)]
    pub(crate) local_port: u16,
}
