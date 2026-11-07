//! One test binary for the whole public API, so it links once.

mod collection;
mod enum_filter;
mod extract;

use anyhow::Result;
use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::lifecycle::{Add, Discard, Insert, Remove};
use bevy_ecs::observer::On;
use bevy_ecs::resource::Resource;
use bevy_ecs::schedule::{Schedule, ScheduleLabel};
use bevy_ecs::system::ResMut;

/// Lifecycle events observed for one component type.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Transitions {
    add: usize,
    insert: usize,
    discard: usize,
    remove: usize,
}

#[derive(Resource)]
struct Tally<C>(Transitions, core::marker::PhantomData<fn() -> C>);

impl Transitions {
    const fn new(add: usize, insert: usize, discard: usize, remove: usize) -> Self {
        Self {
            add,
            insert,
            discard,
            remove,
        }
    }

    /// Starts counting lifecycle events of `C`, read back with [`Self::of`].
    fn observe<C: Component>(app: &mut App) {
        app.insert_resource(Tally::<C>(Self::default(), core::marker::PhantomData))
            .add_observer(|_: On<Add<C>>, mut t: ResMut<Tally<C>>| t.0.add += 1)
            .add_observer(|_: On<Insert<C>>, mut t: ResMut<Tally<C>>| t.0.insert += 1)
            .add_observer(|_: On<Discard<C>>, mut t: ResMut<Tally<C>>| t.0.discard += 1)
            .add_observer(|_: On<Remove<C>>, mut t: ResMut<Tally<C>>| t.0.remove += 1);
    }

    fn of<C: Component>(app: &App) -> Result<Self> {
        app.world()
            .get_resource::<Tally<C>>()
            .map(|tally| tally.0)
            .ok_or_else(|| anyhow::anyhow!("call Transitions::observe first"))
    }
}

fn systems_in(app: &App, label: impl ScheduleLabel) -> usize {
    app.get_schedule(label).map_or(0, Schedule::systems_len)
}

fn has<C: Component>(app: &App, entity: Entity) -> Result<bool> {
    Ok(app.world().get_entity(entity)?.contains::<C>())
}

fn get<C: Component + Clone>(app: &App, entity: Entity) -> Result<Option<C>> {
    Ok(app.world().get_entity(entity)?.get::<C>().cloned())
}
