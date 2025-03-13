// wait_ext.rs
use async_trait::async_trait;
use futures::{Future, StreamExt};
use libp2p::swarm::{Swarm, SwarmEvent};
use std::time::Duration;
use tracing::debug;

/// A trait that extends Swarm with methods to wait for specific events
#[async_trait(?Send)] // Note the ?Send here to remove Send requirement
pub trait SwarmWaitExt {
    type Event: std::fmt::Debug;

    /// Wait for a specific event matching the provided predicate
    async fn wait_for<F, R>(&mut self, predicate: F) -> R
    where
        F: FnMut(&mut Self, &Self::Event) -> Option<R>; // Removed Send bound

    /// Wait for a specific event matching the provided predicate with a timeout
    async fn wait_for_with_timeout<F, R>(
        &mut self,
        predicate: F,
        timeout: Duration,
    ) -> Result<R, tokio::time::error::Elapsed>
    where
        F: FnMut(&mut Self, &Self::Event) -> Option<R>; // Removed Send bound
}

#[async_trait(?Send)] // Note the ?Send here to remove Send requirement
impl<B> SwarmWaitExt for Swarm<B>
where
    B: libp2p::swarm::NetworkBehaviour,
    B::ToSwarm: std::fmt::Debug,
{
    // Use the correct SwarmEvent type with a single generic parameter
    type Event = SwarmEvent<B::ToSwarm>;

    async fn wait_for<F, R>(&mut self, mut predicate: F) -> R
    where
        F: FnMut(&mut Self, &Self::Event) -> Option<R>, // Removed Send bound
    {
        loop {
            if let Some(event) = self.next().await {
                if let Some(result) = predicate(self, &event) {
                    return result;
                }

                debug!(?event, "Other event while waiting for predicate");
            }
        }
    }

    async fn wait_for_with_timeout<F, R>(
        &mut self,
        mut predicate: F,
        timeout: Duration,
    ) -> Result<R, tokio::time::error::Elapsed>
    where
        F: FnMut(&mut Self, &Self::Event) -> Option<R>, // Removed Send bound
    {
        tokio::time::timeout(timeout, async move {
            loop {
                if let Some(event) = self.next().await {
                    if let Some(result) = predicate(self, &event) {
                        return result;
                    }
                    // Skip debug logging to avoid Debug trait requirements
                }
            }
        })
        .await
    }
}
