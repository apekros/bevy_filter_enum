//! Entity mutations behind the generated [`EnumFilter::sync_markers`].
//!
//! Every structural change on an [`EntityWorldMut`] flushes the world, so
//! observers reacting to a marker can despawn the entity halfway through a
//! sync. Each step therefore checks [`EntityWorldMut::is_despawned`] before
//! touching the entity again, which is the guard Bevy documents for exactly
//! this case.
//!
//! Markers already in the right state are left alone, so `Add` and `Remove`
//! fire only on real variant transitions.

use bevy_ecs::component::Component;
use bevy_ecs::world::EntityWorldMut;

use crate::EnumFilter;

/// Makes `M` the only marker of `T` on the entity.
pub fn set_variant<T, M>(
    entity: &mut EntityWorldMut<'_>,
    remove_stale_payloads: impl FnOnce(&mut EntityWorldMut<'_>),
) where
    T: EnumFilter,
    M: Component + Default,
{
    if entity.contains::<M>() {
        return;
    }
    if leave_previous_variant::<T>(entity, remove_stale_payloads) {
        entity.insert(M::default());
    }
}

/// Like [`set_variant`], and mirrors the variant's payload as a component.
///
/// Within the same variant the payload is replaced in place (`Discard` then
/// `Insert`, no `Remove`/`Add`). On a transition the marker and payload land
/// in one insert, so an observer on either already sees both.
pub fn set_variant_with_payload<T, M, P>(
    entity: &mut EntityWorldMut<'_>,
    payload: P,
    remove_stale_payloads: impl FnOnce(&mut EntityWorldMut<'_>),
) where
    T: EnumFilter,
    M: Component + Default,
    P: Component,
{
    if entity.contains::<M>() {
        entity.insert(payload);
        return;
    }
    if leave_previous_variant::<T>(entity, remove_stale_payloads) {
        entity.insert((M::default(), payload));
    }
}

/// Strips every marker of `T` and every extracted payload.
pub fn clear<T: EnumFilter>(
    entity: &mut EntityWorldMut<'_>,
    remove_payloads: impl FnOnce(&mut EntityWorldMut<'_>),
) {
    leave_previous_variant::<T>(entity, remove_payloads);
}

/// Removes one extracted payload type, unless an observer already despawned
/// the entity.
pub fn remove<P: Component>(entity: &mut EntityWorldMut<'_>) {
    if !entity.is_despawned() {
        entity.remove::<P>();
    }
}

/// Compile-time check that an extracted payload can be mirrored. Its only job
/// is to put the error on the payload type instead of deep in generated code.
pub fn assert_payload<P: Component + Clone>() {}

/// Removes the old marker and payloads. Returns whether the entity survived
/// the observers that fired along the way.
fn leave_previous_variant<T: EnumFilter>(
    entity: &mut EntityWorldMut<'_>,
    remove_stale_payloads: impl FnOnce(&mut EntityWorldMut<'_>),
) -> bool {
    entity.remove::<T::Markers>();
    remove_stale_payloads(entity);
    !entity.is_despawned()
}
