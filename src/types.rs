use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Config {
    token: String,
    guild_id: String,
    delegate_role_id: String,
    staff_role_id: String,
    chair_role_id: String,
    committees: Vec<Committee>,
}

impl Config {
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn guild_id(&self) -> u64 {
        self.guild_id.parse().unwrap()
    }

    pub fn delegate_role_id(&self) -> u64 {
        self.delegate_role_id.parse().unwrap()
    }

    pub fn staff_role_id(&self) -> u64 {
        self.staff_role_id.parse().unwrap()
    }

    pub fn chair_role_id(&self) -> u64 {
        self.chair_role_id.parse().unwrap()
    }

    pub fn committees(&self) -> &[Committee] {
        &self.committees
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Committee {
    name: String,
    role_id: String,
    channel_id: String,
}

impl Committee {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn role_id(&self) -> u64 {
        self.role_id.parse().unwrap()
    }

    pub fn channel_id(&self) -> u64 {
        self.channel_id.parse().unwrap()
    }
}
