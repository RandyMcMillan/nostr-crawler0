use nostr_relays::processor::Processor;
use nostr_relays::processor::APP_SECRET_KEY;
use nostr_relays::processor::BOOTSTRAP_RELAY1;
use nostr_relays::processor::BOOTSTRAP_RELAY2;
use nostr_relays::processor::BOOTSTRAP_RELAY3;
use nostr_relays::relay_manager::RelayManager;
use nostr_sdk::prelude::{FromBech32, Keys, Result, SecretKey};

#[tokio::main]
async fn main() -> Result<()> {
    let app_secret_key = SecretKey::from_bech32(APP_SECRET_KEY)?;
    let app_keys = Keys::new(app_secret_key);
    let processor = Processor::default();
    let mut relay_manager = RelayManager::new(app_keys, processor);
    relay_manager
        .run(vec![BOOTSTRAP_RELAY1, BOOTSTRAP_RELAY2, BOOTSTRAP_RELAY3])
        .await?;
    relay_manager.processor.dump();

    Ok(())
}
