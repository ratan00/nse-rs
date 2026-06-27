use nse_rs::NseClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Initializing NseClient...");
    let client = NseClient::new();
    client.init_session().await?;
    println!("Session initialized.");

    println!("\n--- Test 1: get_index_quote(\"NIFTY 50\") ---");
    match client.get_index_quote("NIFTY 50").await {
        Ok(q) => println!("Success! Quote: {:?}", q),
        Err(e) => println!("Failed: {:?}", e),
    }

    println!("\n--- Test 2: get_index_quote(\"NIFTY BANK\") ---");
    match client.get_index_quote("NIFTY BANK").await {
        Ok(q) => println!("Success! Quote: {:?}", q),
        Err(e) => println!("Failed: {:?}", e),
    }

    Ok(())
}
