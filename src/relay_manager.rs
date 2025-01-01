use crate::processor::Processor;
use crate::relays::Relays;
use nostr_sdk::{
    prelude::{
        Client, Event, Filter, Keys, Kind, Options, RelayPoolNotification, Result, Tag, Timestamp,
        Url,
    },
    RelayMessage, RelayStatus,
};
use std::collections::HashSet;
use std::time::Duration;

const MAX_ACTIVE_RELAYS: usize = 50;
const PERIOD_START_PAST_SECS: u64 = 6 * 60 * 60;

/// Keeps a set of active connections to relays
pub struct RelayManager {
    // app_keys: Keys,
    relays: Relays,
    relay_client: Client,
    pub processor: Processor,
    /// Time of last event seen (real time, Unix timestamp)
    time_last_event: u64,
}

impl RelayManager {
    pub fn new(app_keys: Keys, processor: Processor) -> Self {
        let opts = Options::new(); //.wait_for_send(true);
        let relay_client = Client::new_with_opts(&app_keys, opts);
        Self {
            // app_keys,
            relays: Relays::default(),
            relay_client,
            processor,
            time_last_event: Self::now(),
        }
    }

    fn add_bootstrap_relays_if_needed(&mut self, bootstrap_relays: Vec<&str>) {
        for us in &bootstrap_relays {
            if self.relays.count() >= MAX_ACTIVE_RELAYS {
                return;
            }
            self.relays.add(us);
        }
    }

    async fn add_some_relays(&mut self) -> Result<()> {
        // remove all
        loop {
            let relays = self.relay_client.relays().await;
            let relay_urls: Vec<&Url> = relays.keys().collect();
            if relay_urls.is_empty() {
                break;
            }
            self.relay_client
                .remove_relay(relay_urls[0].to_string())
                .await?;
        }
        let some_relays = self.relays.get_some(MAX_ACTIVE_RELAYS);
        for r in some_relays {
            self.relay_client.add_relay(r, None).await?;
        }
        Ok(())
    }

    pub async fn run(&mut self, bootstrap_relays: Vec<&str>) -> Result<()> {
        self.add_bootstrap_relays_if_needed(bootstrap_relays);
        self.add_some_relays().await?;
        let some_relays = self.relays.get_some(MAX_ACTIVE_RELAYS);
        for url in &some_relays {
            self.relay_client.add_relay(url.to_string(), None).await?;
        }
        self.connect().await?;

        self.wait_and_handle_messages().await?;

        //println!("STOPPED");
        //println!("======================================================");
        //println!();
        self.relays.dump();

        Ok(())
    }

    async fn connect(&mut self) -> Result<()> {
        let relays = self.relay_client.relays().await;
        //println!("Connecting to {} relays ...", relays.len());
        for u in relays.keys() {
            //print!("{:?} ", u.to_string())
        }
        //println!();
        // Warning: error is not handled here, should check back status
        self.relay_client.connect().await;
        //println!("Connected");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.relay_client.disconnect().await?;
        //println!("Disconnected");
        Ok(())
    }

    async fn subscribe(&mut self, time_start: Timestamp, time_end: Timestamp) -> Result<()> {
        self.relay_client
            .subscribe(vec![Filter::new()
                // .pubkey(keys.public_key())
                // .kind(Kind::RecommendRelay)
                .kinds(vec![Kind::ContactList, Kind::RecommendRelay])
                .since(time_start)
                .until(time_end)])
            .await;
        //println!("Subscribed to relay events",);
        Ok(())
    }

    async fn unsubscribe(&mut self) -> Result<()> {
        self.relay_client.unsubscribe().await;
        //println!("Unsubscribed from relay events ...");
        Ok(())
    }

    async fn reconnect(&mut self) -> Result<()> {
        let connected_relays = self.relay_client.relays().await.len();
        let available_relays = self.relays.count();
        if connected_relays < MAX_ACTIVE_RELAYS && available_relays > connected_relays {
            //println!("Reconnect {} {}", connected_relays, available_relays);
            self.disconnect().await?;
            self.add_some_relays().await?;
            self.connect().await?;
        }
        Ok(())
    }

    async fn wait_and_handle_messages(&mut self) -> Result<()> {
        // Keep track of relays with EOSE sent
        let mut eose_relays = HashSet::<Url>::new();

        let now = Timestamp::now();
        let period_end = now;
        let period_start = period_end - Duration::from_secs(PERIOD_START_PAST_SECS);
        self.subscribe(period_start, period_end).await?;

        let mut notifications = self.relay_client.notifications();
        while let Ok(notification) = notifications.recv().await {
            //println!("relaynotif {:?}", notification);
            match notification {
                RelayPoolNotification::Event(_url, event) => {
                    self.handle_event(&event);
                    // invoke callback
                    self.processor.handle_event(&event);
                }
                RelayPoolNotification::Message(url, relaymsg) => match relaymsg {
                    RelayMessage::EndOfStoredEvents(_sub_id) => {
                        eose_relays.insert(url.clone());
                        let n1 = eose_relays.len();
                        let n2 = self.relay_client.relays().await.len();
                        let mut n_connected = 0;
                        let mut n_connecting = 0;
                        let relays = self.relay_client.relays().await;
                        for relay in relays.values() {
                            match relay.status().await {
                                RelayStatus::Connected => n_connected += 1,
                                RelayStatus::Connecting => n_connecting += 1,
                                _ => {}
                            }
                        }
                        //println!("Received EOSE from {url}, total {n1} ({n2} relays, {n_connected} connected {n_connecting} connecting)");

                        // Check for stop: All connected/connecting relays have signalled EOSE, or
                        if n1 >= (n_connected + n_connecting) && (n_connected + n_connecting > 0) {
                            //println!("STOPPING; All relays signalled EOSE ({n1})");
                            break;
                        }
                    }
                    RelayMessage::Event {
                        subscription_id: _,
                        event: _,
                    } => {}
                    _ => {
                        //println!("{{\"{:?}\":\"{url}\"}}", relaymsg);
                    }
                },
                RelayPoolNotification::Shutdown => break,
            }
            // Check for stop: There was no event in the last few seconds, and there were some EOSE already
            let last_age = self.get_last_event_ago();
            let n1 = eose_relays.len();
            if last_age > 20 && n1 >= 2 {
                //println!(
                //    "STOPPING; There were some EOSE-s, and no events in the past {} secs",
                //    last_age
                //);
                break;
            }

            self.reconnect().await?;
        }
        self.unsubscribe().await?;
        self.disconnect().await?;
        Ok(())
    }

    fn handle_event(&mut self, event: &Event) {
        match event.kind {
            Kind::Metadata => {
                println!("{:?}", event.kind);
            }
            Kind::TextNote => {
                println!("{:?}", event.kind);
            }
            Kind::EncryptedDirectMessage => {
                println!("{:?}", event.kind);
            }
            Kind::EventDeletion => {
                println!("{:?}", event.kind);
            }
            Kind::Repost => {
                println!("{:?}", event.kind);
            }
            Kind::Reaction => {
                println!("{:?}", event.kind);
            }
            Kind::ChannelCreation => {
                println!("{:?}", event.kind);
            }
            Kind::ChannelMetadata => {
                println!("{:?}", event.kind);
            }
            Kind::ChannelMessage => {
                println!("{:?}", event.kind);
            }
            Kind::ChannelHideMessage => {
                println!("{:?}", event.kind);
            }
            Kind::ChannelMuteUser => {
                println!("{:?}", event.kind);
            }
            Kind::PublicChatReserved45 => {
                println!("{:?}", event.kind);
            }
            Kind::PublicChatReserved46 => {
                println!("{:?}", event.kind);
            }
            Kind::PublicChatReserved47 => {
                println!("{:?}", event.kind);
            }
            Kind::PublicChatReserved48 => {
                println!("{:?}", event.kind);
            }
            Kind::PublicChatReserved49 => {
                println!("{:?}", event.kind);
            }
            Kind::Reporting => {
                println!("{:?}", event.kind);
            }
            Kind::ZapRequest => {
                println!("{:?}", event.kind);
            }
            Kind::Zap => {
                println!("{:?}", event.kind);
            }
            Kind::Authentication => {
                println!("{:?}", event.kind);
            }
            Kind::NostrConnect => {
                println!("{:?}", event.kind);
            }
            Kind::LongFormTextNote => {
                println!("{:?}", event.kind);
                self.update_event_time();
                // count p tags
                let mut cnt = 0;
                for t in &event.tags {
                    if let Tag::PubKey(_pk, s) = t {
                        // state.pubkeys.add(pk);
                        if let Some(ss) = s {
                            //println!("    {ss}");
                            let _ = self.relays.add(ss);
                        }
                        cnt += 1;
                    }
                }
            }
            Kind::RelayList => {
                println!("{:?}", event.kind);
            }
            Kind::Replaceable(u16) => {
                println!("{:?}", event.kind);
            }
            Kind::Ephemeral(u16) => {
                println!("{:?}", event.kind);
            }
            Kind::ParameterizedReplaceable(u16) => {
                println!("{:?}", event.kind);
            }
            Kind::Custom(u64) => {
                println!("{:?}", event.kind);
            }
            Kind::ContactList => {
                self.update_event_time();
                // count p tags
                let mut cnt = 0;
                for t in &event.tags {
                    if let Tag::PubKey(_pk, s) = t {
                        // state.pubkeys.add(pk);
                        if let Some(ss) = s {
                            //println!("    {ss}");
                            let _ = self.relays.add(ss);
                        }
                        cnt += 1;
                    }
                }
            }
            Kind::RecommendRelay => {
                self.update_event_time();
                println!("\n318:Relay(s): {}\n", event.content);
                let _ = self.relays.add(&event.content);
            }
            _ => {
                println!("Unsupported event {:?}", event.kind)
            }
        }
    }

    fn update_event_time(&mut self) {
        self.time_last_event = Self::now();
    }

    fn get_last_event_ago(&self) -> u64 {
        Self::now() - self.time_last_event
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }
}
