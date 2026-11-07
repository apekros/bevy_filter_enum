use core::marker::PhantomData;

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::component::{Component, ComponentMutability};
use bevy_ecs::entity::Entity;
use bevy_ecs::intern::Interned;
use bevy_ecs::lifecycle::{Insert, Remove};
use bevy_ecs::observer::On;
use bevy_ecs::query::{Changed, With};
use bevy_ecs::schedule::{IntoScheduleConfigs, ScheduleLabel, SystemSet};
use bevy_ecs::system::{Commands, Query};
use bevy_ecs::world::{EntityWorldMut, World};

use crate::EnumFilter;
use crate::collection::{EnumFilterCollection, sync_collection};

/// Keeps the markers of `T` in sync with the `T` component on each entity.
///
/// Inserting, replacing or removing `T` updates markers right after the
/// change is applied, through observers. If `T` is mutable, edits through
/// `Mut<T>` are invisible to observers, so the plugin also adds a system in
/// [`EnumFilterSystems`] that picks up `Changed<T>`. Mark `T` with
/// `#[component(immutable)]` to get a plugin with no per-frame system at all.
pub struct EnumFilterPlugin<T> {
    schedule: Interned<dyn ScheduleLabel>,
    marker: PhantomData<fn() -> T>,
}

impl<T: EnumFilter + Component> Default for EnumFilterPlugin<T> {
    fn default() -> Self {
        Self::in_schedule(PreUpdate)
    }
}

impl<T: EnumFilter + Component> EnumFilterPlugin<T> {
    /// Runs the `Changed<T>` system in `label` instead of [`PreUpdate`].
    /// Irrelevant for immutable components, which never get that system.
    pub fn in_schedule(label: impl ScheduleLabel) -> Self {
        Self {
            schedule: label.intern(),
            marker: PhantomData,
        }
    }
}

impl<T: EnumFilter + Component> Plugin for EnumFilterPlugin<T> {
    fn build(&self, app: &mut App) {
        add_marker_sync::<EnumSource<T>>(app, self.schedule);
    }
}

/// Keeps the markers of `C::Filter` in sync with the values in collection
/// component `C`. Same lifecycle as [`EnumFilterPlugin`].
///
/// Only one source should drive a given enum's markers on an entity, two
/// collections (or a collection and the enum itself) would fight over them.
pub struct EnumFilterCollectionPlugin<C> {
    schedule: Interned<dyn ScheduleLabel>,
    marker: PhantomData<fn() -> C>,
}

impl<C: EnumFilterCollection> Default for EnumFilterCollectionPlugin<C> {
    fn default() -> Self {
        Self::in_schedule(PreUpdate)
    }
}

impl<C: EnumFilterCollection> EnumFilterCollectionPlugin<C> {
    /// Runs the `Changed<C>` system in `label` instead of [`PreUpdate`].
    /// Irrelevant for immutable components, which never get that system.
    pub fn in_schedule(label: impl ScheduleLabel) -> Self {
        Self {
            schedule: label.intern(),
            marker: PhantomData,
        }
    }
}

impl<C: EnumFilterCollection> Plugin for EnumFilterCollectionPlugin<C> {
    fn build(&self, app: &mut App) {
        add_marker_sync::<CollectionSource<C>>(app, self.schedule);
    }
}

/// Systems that sync markers after in-place edits of mutable components.
///
/// Order systems that filter on markers `.after(EnumFilterSystems)` when they
/// must see edits made through `Mut<T>` earlier in the same frame. Inserts and
/// removals never wait for this set.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnumFilterSystems;

/// A component whose value decides which markers its entity carries.
trait MarkerSource: 'static {
    type Source: Component;

    fn sync(entity: EntityWorldMut<'_>);
}

struct EnumSource<T>(PhantomData<fn() -> T>);

impl<T: EnumFilter + Component> MarkerSource for EnumSource<T> {
    type Source = T;

    fn sync(mut entity: EntityWorldMut<'_>) {
        T::sync_markers::<T>(&mut entity);
    }
}

struct CollectionSource<C>(PhantomData<fn() -> C>);

impl<C: EnumFilterCollection> MarkerSource for CollectionSource<C> {
    type Source = C;

    fn sync(mut entity: EntityWorldMut<'_>) {
        sync_collection::<C>(&mut entity);
    }
}

/// Observers rather than component hooks: a component has one hook slot per
/// lifecycle event and it belongs to the component's author, observers
/// compose with whatever hooks and observers the user already has.
fn add_marker_sync<S: MarkerSource>(app: &mut App, schedule: Interned<dyn ScheduleLabel>) {
    app.add_observer(sync_on_insert::<S>)
        .add_observer(sync_on_remove::<S>);
    if <S::Source as Component>::Mutability::MUTABLE {
        app.add_systems(schedule, sync_changed::<S>.in_set(EnumFilterSystems));
    }
    sync_existing::<S>(app.world_mut());
}

/// Entities spawned before the plugin was added were never observed.
/// Usually there are none and this is one empty query.
fn sync_existing<S: MarkerSource>(world: &mut World) {
    let existing: Vec<Entity> = world
        .query_filtered::<Entity, With<S::Source>>()
        .iter(world)
        .collect();
    for entity in existing {
        if let Ok(entity) = world.get_entity_mut(entity) {
            S::sync(entity);
        }
    }
}

// The sync command re-reads the source component when it runs, so it always
// applies the latest value and is a cheap no-op when markers already match.
// That is what lets every trigger below share one command. It is silenced
// because the entity may legitimately be gone by then (despawn fires
// `Remove`).

fn sync_on_insert<S: MarkerSource>(insert: On<Insert<S::Source>>, mut commands: Commands) {
    commands.entity(insert.entity).queue_silenced(S::sync);
}

fn sync_on_remove<S: MarkerSource>(remove: On<Remove<S::Source>>, mut commands: Commands) {
    commands.entity(remove.entity).queue_silenced(S::sync);
}

fn sync_changed<S: MarkerSource>(
    changed: Query<Entity, Changed<S::Source>>,
    mut commands: Commands,
) {
    for entity in &changed {
        commands.entity(entity).queue_silenced(S::sync);
    }
}
