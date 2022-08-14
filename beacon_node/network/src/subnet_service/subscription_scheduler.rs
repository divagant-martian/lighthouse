use delay_map::{HashMapDelay, HashSetDelay};
use futures::StreamExt;
use std::{
    collections::VecDeque,
    marker::PhantomData,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use slot_clock::SlotClock;
use types::{Slot, SubnetId};

/// The minimum number of slots ahead that we attempt to discover peers for a subscription.
/// If the slot is less than this number, skip the peer discovery process.
const MIN_PEER_DISCOVERY_SLOT_LOOK_AHEAD: u64 = 2;

/// Events emitted by the [`SubscriptionScheduler`]
#[derive(Debug, PartialEq)]
pub enum Event {
    /// We now consider ourselves subscribed to this subnet.
    Subscribe(SubnetId),
    /// We no longer consider ourselves subscribed to this subnet.
    Unsubscribe(SubnetId),
    /// We should find peers subscribed to this subnet.
    FindPeers {
        subnet_id: SubnetId,
        min_ttl: std::time::Instant,
    },
}

/// Scheduler for subnet subscriptions in a slot scale.
#[allow(dead_code)]
pub struct Subscriptions<S> {
    /// Subnets currently subscribed to and the slot in which their subscription ends.
    current_subscriptions: HashMapDelay<SubnetId, Slot>,
    /// For how many slots must each subscription last.
    subscription_duration_slots: u64,
    /// Subscriptions that need to be done in the future.
    scheduled_subscriptions: HashSetDelay<(SubnetId, Slot)>,
    /// When scheduling a subscription that must be ready in certain slot, the scheduled time will
    /// be advanced using this field.
    advance_subscription_duration: Duration,
    /// Whether we send out events to discover peers. Peer discovery will only be done for
    /// subscriptions far enough in the future, according to `MIN_PEER_DISCOVERY_SLOT_LOOK_AHEAD`.
    discovery_enabled: bool,
    /// Events to emit when a subscription starts and ends being active.
    events: VecDeque<Event>,
    /// Waker for this task.
    waker: Option<Waker>,
    /// Phantom for the SlotClock type.
    _phantom: PhantomData<S>,
}

impl<S: SlotClock> Subscriptions<S> {
    pub fn new(
        subscription_duration_slots: u64,
        advance_subscription_duration: Duration,
        discovery_enabled: bool,
        slot_clock: &S,
    ) -> Self {
        let default_subscription_duration_millis = Duration::from_millis(
            slot_clock.slot_duration().as_millis() as u64 * subscription_duration_slots,
        );
        let default_scheduling_duration = slot_clock.slot_duration();
        Subscriptions {
            current_subscriptions: HashMapDelay::new(default_subscription_duration_millis),
            subscription_duration_slots,
            scheduled_subscriptions: HashSetDelay::new(default_scheduling_duration),
            advance_subscription_duration,
            discovery_enabled,
            events: Default::default(),
            waker: Default::default(),
            _phantom: Default::default(),
        }
    }

    // Subscribes to the subnet if it should be done immediately, or schedules it if required.
    pub fn subscribe_or_schedule(
        &mut self,
        subnet_id: SubnetId,
        start_slot: Slot,
        slot_clock: &S,
    ) -> Result<(), &'static str> {
        // Calculate how long before we need to subscribe to the subnet.
        let time_to_subscription_start = {
            // The short time we schedule the subscription before it's actually required. This
            // ensures we are subscribed on time, and allows consecutive subscriptions to the same
            // subnet to overlap, reducing subnet churn.
            // The time to the required slot.
            let time_to_subscription_slot =
                slot_clock.duration_to_slot(start_slot).unwrap_or_default();
            time_to_subscription_slot.saturating_sub(self.advance_subscription_duration)
        };

        // If the subscription should be done in the future, schedule it. Otherwise subscribe
        // immediately.
        if time_to_subscription_start.is_zero() {
            // This is a current or past slot, we subscribe immediately.
            self.subscribe_to_subnet_immediately(
                subnet_id,
                start_slot + self.subscription_duration_slots,
                slot_clock,
            )?;
        } else {
            // This is a future slot, schedule subscribing.
            self.schedule_subscription(
                subnet_id,
                start_slot,
                time_to_subscription_start,
                slot_clock,
            )?;
        }

        Ok(())
    }

    /// Returns whether there is an active subscription for this subnet.
    pub fn is_subscribed(&self, subnet_id: &SubnetId) -> bool {
        self.current_subscriptions.contains_key(subnet_id)
    }

    /// Gives access to the active subscriptions.
    pub fn current_subscriptions(&self) -> impl Iterator<Item = &SubnetId> {
        self.current_subscriptions.keys()
    }

    /// Registers a subnet as subscribed.
    ///
    /// Checks that the time in which the subscription would end is not in the past. If we are
    /// already subscribed, extends the timeout if necessary. If this is a new subscription, we send
    /// out the appropriate events.
    fn subscribe_to_subnet_immediately(
        &mut self,
        subnet_id: SubnetId,
        end_slot: Slot,
        slot_clock: &S,
    ) -> Result<(), &'static str> {
        let time_to_subscription_end = slot_clock.duration_to_slot(end_slot).unwrap_or_default();

        // First check this is worth doing.
        if time_to_subscription_end.is_zero() {
            return Err("Time when subscription would end has already passed.");
        }

        match self.current_subscriptions.get(&subnet_id) {
            Some(current_end_slot) => {
                // We are already subscribed. Check if we need to extend the subscription.
                if &end_slot > current_end_slot {
                    self.current_subscriptions.insert_at(
                        subnet_id,
                        end_slot,
                        time_to_subscription_end,
                    );
                }
            }
            None => {
                // This is a new subscription. Add with the corresponding timeout and send the
                // notification.
                self.current_subscriptions
                    .insert_at(subnet_id, end_slot, time_to_subscription_end);

                self.queue_event(Event::Subscribe(subnet_id));
            }
        }

        Ok(())
    }

    /// Adds an event to the event queue and notifies that this task is ready to be polled
    /// again.
    fn queue_event(&mut self, ev: Event) {
        self.events.push_back(ev);
        if let Some(waker) = &self.waker {
            waker.wake_by_ref()
        }
    }

    /// [`futures::Stream`] like implementation for [`SubscriptionScheduler`].
    pub fn poll_next(
        &mut self,
        cx: &mut Context<'_>,
        slot_clock: &S,
    ) -> Poll<Result<Event, String>> {
        // Update the waker if needed.
        if let Some(waker) = &self.waker {
            if waker.will_wake(cx.waker()) {
                self.waker = Some(cx.waker().clone());
            }
        } else {
            self.waker = Some(cx.waker().clone());
        }

        // Send out any generated events.
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(Ok(event));
        }

        // Process scheduled subscriptions that might be ready, since those can extend a soon to
        // expire subscription.
        match self.scheduled_subscriptions.poll_next_unpin(cx) {
            Poll::Ready(Some(Ok((subnet_id, slot)))) => {
                if let Err(e) = self.subscribe_to_subnet_immediately(
                    subnet_id,
                    slot + self.subscription_duration_slots,
                    slot_clock,
                ) {
                    return Poll::Ready(Err(e.to_string()));
                }
            }
            Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(e)),
            Poll::Ready(None) | Poll::Pending => {}
        }

        // Finally process any expired subscriptions.
        match self.current_subscriptions.poll_next_unpin(cx) {
            Poll::Ready(Some(Ok((subnet_id, _slot)))) => {
                return Poll::Ready(Ok(Event::Unsubscribe(subnet_id)))
            }
            Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(e)),
            Poll::Ready(None) | Poll::Pending => {}
        }

        Poll::Pending
    }

    /// Schedules subscribing to a subnet in the future. If there is enough time for a peer
    /// discovery it queues the corresponding event.
    fn schedule_subscription(
        &mut self,
        subnet_id: SubnetId,
        start_slot: Slot,
        time_to_subscription_start: Duration,
        slot_clock: &S,
    ) -> Result<(), &'static str> {
        self.scheduled_subscriptions
            .insert_at((subnet_id, start_slot), time_to_subscription_start);
        if self.discovery_enabled {
            if time_to_subscription_start.as_millis()
                >= slot_clock.slot_duration().as_millis()
                    * MIN_PEER_DISCOVERY_SLOT_LOOK_AHEAD as u128
            {
                let time_to_subscription_end = slot_clock
                    .duration_to_slot(start_slot + self.subscription_duration_slots)
                    .ok_or("Failed to get duration to end of subscription slot")?;
                let min_ttl = Instant::now() + time_to_subscription_end;
                self.queue_event(Event::FindPeers { subnet_id, min_ttl })
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::time::SystemTime;

    use slot_clock::SystemTimeSlotClock;

    use super::*;

    const SLOT_DURATION_MILLIS: u64 = 100;
    const SUBSCRIPTION_DURATION_SLOTS: u64 = 1;
    // a third of a slot duration.
    const ADVANCE_SUBSCRIPTION_DURATION: Duration = Duration::from_millis(SLOT_DURATION_MILLIS / 3);

    fn recent_genesis_time() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn slot_clock() -> SystemTimeSlotClock {
        SystemTimeSlotClock::new(
            Slot::new(0),
            Duration::from_secs(recent_genesis_time()),
            Duration::from_millis(SLOT_DURATION_MILLIS),
        )
    }

    impl Subscriptions<SystemTimeSlotClock> {
        fn new_for_testing(slot_clock: &SystemTimeSlotClock) -> Self {
            const _: () =
                if ADVANCE_SUBSCRIPTION_DURATION.as_millis() >= SLOT_DURATION_MILLIS as u128 {
                    panic!("Tests assume an advance_subscription_duration of less than a slot.");
                } else if ADVANCE_SUBSCRIPTION_DURATION.is_zero() {
                    panic!("Tests assume advance_subscription_duration is positive.");
                };
            let discovery_enabled = false;
            Subscriptions::new(
                SUBSCRIPTION_DURATION_SLOTS,
                ADVANCE_SUBSCRIPTION_DURATION,
                discovery_enabled,
                slot_clock,
            )
        }
    }

    /// Testing aid to implement Stream over SubscriptionScheduler
    struct T<'a> {
        scheduler: &'a mut Subscriptions<SystemTimeSlotClock>,
        slot_clock: &'a SystemTimeSlotClock,
    }

    impl<'a> futures::Stream for T<'a> {
        type Item = Result<Event, String>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let T {
                scheduler,
                slot_clock,
            } = self.get_mut();
            match scheduler.poll_next(cx, slot_clock) {
                Poll::Ready(e) => Poll::Ready(Some(e)),
                Poll::Pending => Poll::Pending,
            }
        }
    }

    async fn get_events<'a>(
        scheduler: &'a mut Subscriptions<SystemTimeSlotClock>,
        slot_clock: &'a SystemTimeSlotClock,
        until_slot: Slot,
    ) -> Result<Vec<Event>, String> {
        let deadline = tokio::time::sleep(slot_clock.duration_to_slot(until_slot).unwrap());
        futures::pin_mut!(deadline);
        let mut t = T {
            scheduler,
            slot_clock,
        };

        let mut events = Vec::default();
        loop {
            tokio::select! {
                Some(event) = t.next() => {
                    events.push(event);
                }
                _ = deadline.as_mut() => {
                    break;
                }
            }
        }

        events.into_iter().collect()
    }

    /// Tests that we subscribe to a subnet during the right slot taking into account the
    /// `ADVANCE_SUBSCRIPTION_DURATION`, remain subscribed during that slot and correctly
    /// unsubscribe when required.
    #[tokio::test]
    async fn subscribe_wait_for_unsubscribe() {
        let slot_clock = slot_clock();
        let mut scheduler = Subscriptions::new_for_testing(&slot_clock);
        let current_slot = slot_clock.now().expect("Could not get current slot");

        // slots before the subscription
        const WAIT_SLOTS: u64 = 3;

        let start_slot = current_slot + WAIT_SLOTS;
        let end_slot = start_slot + SUBSCRIPTION_DURATION_SLOTS;
        let subnet_id = SubnetId::new(10);

        // submit the subscriptions
        scheduler
            .subscribe_or_schedule(subnet_id, start_slot, &slot_clock)
            .unwrap();

        let no_events = get_events(&mut scheduler, &slot_clock, start_slot - 1)
            .await
            .unwrap();
        assert_eq!(no_events[..], []);

        let subscription = get_events(&mut scheduler, &slot_clock, start_slot)
            .await
            .unwrap();

        assert_eq!(subscription[..], [Event::Subscribe(subnet_id)]);

        assert!(scheduler.is_subscribed(&subnet_id));
        assert_eq!(scheduler.current_subscriptions().count(), 1);

        let unsubscription = get_events(&mut scheduler, &slot_clock, end_slot + 1)
            .await
            .unwrap();

        assert_eq!(unsubscription[..], [Event::Unsubscribe(subnet_id)]);

        assert!(!scheduler.is_subscribed(&subnet_id));
        assert_eq!(scheduler.current_subscriptions().count(), 0);
    }

    /// Checks that consecutive subscriptions don't produce unsubscribe events immediately
    /// followed by subscription events.
    #[tokio::test]
    async fn subscribe_consecutive_slots() {
        let slot_clock = slot_clock();
        let mut scheduler = Subscriptions::new_for_testing(&slot_clock);

        let current_slot = slot_clock.now().expect("Could not get current slot");
        let subnet_id = SubnetId::new(10);

        let start_slot1 = current_slot + 2;
        let start_slot2 = current_slot + 3;

        // submit the subscription
        scheduler
            .subscribe_or_schedule(subnet_id, start_slot1, &slot_clock)
            .unwrap();

        scheduler
            .subscribe_or_schedule(subnet_id, start_slot2, &slot_clock)
            .unwrap();

        let subscription = get_events(&mut scheduler, &slot_clock, start_slot1)
            .await
            .unwrap();

        assert_eq!(subscription[..], [Event::Subscribe(subnet_id)]);

        let no_events = get_events(
            &mut scheduler,
            &slot_clock,
            start_slot2 + SUBSCRIPTION_DURATION_SLOTS - 1,
        )
        .await
        .unwrap();

        assert_eq!(no_events, []);

        assert!(scheduler.is_subscribed(&subnet_id));
        assert_eq!(scheduler.current_subscriptions().count(), 1);

        let unsubscription = get_events(
            &mut scheduler,
            &slot_clock,
            start_slot2 + SUBSCRIPTION_DURATION_SLOTS + 1,
        )
        .await
        .unwrap();

        assert_eq!(unsubscription[..], [Event::Unsubscribe(subnet_id)]);

        assert!(!scheduler.is_subscribed(&subnet_id));
        assert_eq!(scheduler.current_subscriptions().count(), 0);
    }

    /// Tests that two non-consecutive subscriptions are handled correctly.
    #[tokio::test]
    async fn subscribe_several_slots_apart() {
        let slot_clock = slot_clock();
        let mut scheduler = Subscriptions::new_for_testing(&slot_clock);

        let current_slot = slot_clock.now().expect("Could not get current slot");
        let subnet_id = SubnetId::new(10);

        let start_slot1 = current_slot + 2;
        let start_slot2 = start_slot1 + SUBSCRIPTION_DURATION_SLOTS + 2;

        // submit the subscriptions
        scheduler
            .subscribe_or_schedule(subnet_id, start_slot1, &slot_clock)
            .unwrap();

        scheduler
            .subscribe_or_schedule(subnet_id, start_slot2, &slot_clock)
            .unwrap();

        // Subscription and unsubscription for the first submission.
        let subscription = get_events(&mut scheduler, &slot_clock, start_slot1)
            .await
            .unwrap();

        assert_eq!(subscription[..], [Event::Subscribe(subnet_id)]);

        let unsubscription = get_events(
            &mut scheduler,
            &slot_clock,
            start_slot1 + SUBSCRIPTION_DURATION_SLOTS + 1,
        )
        .await
        .unwrap();

        assert_eq!(unsubscription[..], [Event::Unsubscribe(subnet_id)]);

        // Subscription and unsubscription for the second submission.
        let subscription = get_events(&mut scheduler, &slot_clock, start_slot2)
            .await
            .unwrap();

        assert_eq!(subscription[..], [Event::Subscribe(subnet_id)]);

        let unsubscription = get_events(
            &mut scheduler,
            &slot_clock,
            start_slot2 + SUBSCRIPTION_DURATION_SLOTS + 1,
        )
        .await
        .unwrap();

        assert_eq!(unsubscription[..], [Event::Unsubscribe(subnet_id)]);
    }
}
