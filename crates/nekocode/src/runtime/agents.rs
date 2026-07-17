use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::agent::Agent;

use super::RuntimeError;

#[derive(Default)]
pub(crate) struct AgentRegistry {
    agents: DashMap<u64, Arc<Agent>>,
}

impl AgentRegistry {
    pub(crate) fn get(&self, thread_id: u64) -> Option<Arc<Agent>> {
        self.agents
            .get(&thread_id)
            .map(|entry| entry.value().clone())
    }

    pub(crate) fn contains(&self, thread_id: u64) -> bool {
        self.agents.contains_key(&thread_id)
    }

    pub(crate) fn activate_new(
        &self,
        thread_id: u64,
        agent: Agent,
    ) -> Result<Arc<Agent>, RuntimeError> {
        match self.agents.entry(thread_id) {
            dashmap::mapref::entry::Entry::Occupied(_) => Err(RuntimeError::ThreadAlreadyActivated),
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                let agent = Arc::new(agent);
                entry.insert(agent.clone());
                Ok(agent)
            }
        }
    }

    pub(crate) fn activate_or_get(&self, thread_id: u64, agent: Agent) -> Arc<Agent> {
        match self.agents.entry(thread_id) {
            dashmap::mapref::entry::Entry::Occupied(entry) => entry.get().clone(),
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                let agent = Arc::new(agent);
                entry.insert(agent.clone());
                agent
            }
        }
    }

    pub(crate) async fn remove_and_shutdown(&self, thread_id: u64) -> bool {
        let Some((_, agent)) = self.agents.remove(&thread_id) else {
            return false;
        };
        agent.shutdown().await;
        true
    }
}
