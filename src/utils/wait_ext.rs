// wait_ext.rs
use async_trait::async_trait;
use futures::{Future, StreamExt};
use libp2p::swarm::{Swarm, SwarmEvent};
use std::pin::Pin; // Add this import
use std::time::Duration;
use tracing::debug;

/// A trait that extends Swarm with methods to wait for specific events
pub trait SwarmWaitExt {
    type Event: std::fmt::Debug;

    /// Wait for a specific event matching the provided predicate
    fn wait_for<F, R>(&mut self, predicate: F) -> Pin<Box<dyn Future<Output = R> + Send + '_>>
    where
        F: FnMut(&mut Self, &Self::Event) -> Option<R> + Send + 'static,
        R: 'static;

    /// Wait for a specific event matching the provided predicate with a timeout
    fn wait_for_with_timeout<F, R>(
        &mut self,
        predicate: F,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<R, tokio::time::error::Elapsed>> + Send + '_>>
    where
        F: FnMut(&mut Self, &Self::Event) -> Option<R> + Send + 'static,
        R: 'static;
}

impl<B> SwarmWaitExt for Swarm<B>
where
    B: libp2p::swarm::NetworkBehaviour + Send,
    B::ToSwarm: std::fmt::Debug,
{
    // Use the correct SwarmEvent type with a single generic parameter
    type Event = SwarmEvent<B::ToSwarm>;

    fn wait_for<F, R>(&mut self, mut predicate: F) -> Pin<Box<dyn Future<Output = R> + Send + '_>>
    where
        F: FnMut(&mut Self, &Self::Event) -> Option<R> + Send + 'static,
        R: 'static,
    {
        let mut this = self;
        Box::pin(async move {
            loop {
                if let Some(event) = this.next().await {
                    if let Some(result) = predicate(this, &event) {
                        return result;
                    }

                    debug!(?event, "Other event while waiting for predicate");
                }
            }
        })
    }

    fn wait_for_with_timeout<F, R>(
        &mut self,
        mut predicate: F,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<R, tokio::time::error::Elapsed>> + Send + '_>>
    where
        F: FnMut(&mut Self, &Self::Event) -> Option<R> + Send + 'static,
        R: 'static,
    {
        let mut this = self;
        Box::pin(async move {
            tokio::time::timeout(timeout, async move {
                loop {
                    if let Some(event) = this.next().await {
                        if let Some(result) = predicate(this, &event) {
                            return result;
                        }
                        // Skip debug logging to avoid Debug trait requirements
                    }
                }
            })
            .await
        })
    }
}
