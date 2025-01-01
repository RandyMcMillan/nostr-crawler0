use nostr_sdk::prelude::Url;
use std::collections::HashSet;

/// Maintain a list of all encountered relays
pub struct Relays {
    r: HashSet<Url>,
}

impl Relays {
    pub fn default() -> Self {
        Self {
            r: HashSet::default(),
        }
    }

    pub fn add(&mut self, s1: &str) -> bool {
        let mut res = false;
        if let Ok(u) = Url::parse(s1) {
            res = self.r.insert(u);
            if res {
                self.print();
            }
        }
        res
    }

    pub fn count(&self) -> usize {
        self.r.len()
    }

    pub fn get_some(&self, max_count: usize) -> Vec<Url> {
        let mut res = Vec::new();
        for u in &self.r {
            res.push(u.clone());
            if res.len() >= max_count {
                return res;
            }
        }
        res
    }

    pub fn print(&self) {
        //println!("43:Relays: {}", self.r.len());
        //print!("    ");
        for u in &self.r {
            //print!("{} ", u.to_string());
        }
        //println!();
    }

    pub fn dump(&self) {
        //println!("Relays: {}", self.r.len());
        print!("[\"RELAYS\",");
        for u in &self.r {
            print!("{{\"{}\"}},", u.to_string());
        }
        print!("{{\"wss://relay.gnostr.org\"}}");
        print!("]");
        //println!();
    }
}
