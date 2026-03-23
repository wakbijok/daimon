use daimon_pve::Client;

#[tokio::main]
async fn main() {
    println!("daimon v{}", env!("CARGO_PKG_VERSION"));

    // Placeholder — will be replaced with config-driven init
    let client = Client::new("https://10.100.10.22:8006", daimon_pve::Auth::Token {
        user: "root@pam".into(),
        token_name: "daimon".into(),
        token_value: "placeholder".into(),
    });

    match client.version().await {
        Ok(version) => println!("Connected to PVE {}", version.version),
        Err(e) => eprintln!("Failed to connect: {e}"),
    }
}
