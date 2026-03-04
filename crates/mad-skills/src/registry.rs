use crate::Skill;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Default)]
pub struct SkillRegistry {
    skills: Arc<RwLock<HashMap<String, Arc<dyn Skill>>>>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, skill: impl Skill + 'static) {
        let mut skills = self.skills.write().unwrap();
        skills.insert(skill.name().to_string(), Arc::new(skill));
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Skill>> {
        let skills = self.skills.read().unwrap();
        skills.get(name).cloned()
    }

    pub fn list(&self) -> Vec<Arc<dyn Skill>> {
        let skills = self.skills.read().unwrap();
        skills.values().cloned().collect()
    }
}
