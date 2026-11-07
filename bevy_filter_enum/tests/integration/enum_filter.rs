use anyhow::Result;
use bevy_app::{App, PreUpdate, Update};
use bevy_ecs::component::{Component, ComponentMutability, StorageType};
use bevy_ecs::lifecycle::Remove;
use bevy_ecs::observer::On;
use bevy_ecs::prelude::{Commands, IntoScheduleConfigs, Query, ResMut, Resource, With};
use bevy_filter_enum::prelude::*;

use crate::{Transitions, has, systems_in};

#[derive(Component, EnumFilter, Debug, Clone, Copy, PartialEq, Eq)]
#[component(immutable)]
enum Mode {
    Idle,
    Run,
}

/// Same shape as [`Mode`], but mutable through `Mut<Gait>`.
#[derive(Component, EnumFilter, Debug, Clone, Copy, PartialEq, Eq)]
enum Gait {
    Walk,
    Sprint,
}

fn app_with<T: EnumFilter + Component>() -> App {
    let mut app = App::new();
    app.add_plugins(EnumFilterPlugin::<T>::default());
    app
}

#[test]
fn spawn_adds_the_active_marker_immediately() -> Result<()> {
    let mut app = app_with::<Mode>();
    let entity = app.world_mut().spawn(Mode::Idle).id();

    assert!(has::<ModeIdle>(&app, entity)?);
    assert!(!has::<ModeRun>(&app, entity)?);
    Ok(())
}

#[test]
fn removing_the_enum_removes_every_marker() -> Result<()> {
    let mut app = app_with::<Mode>();
    let entity = app.world_mut().spawn(Mode::Idle).id();
    app.world_mut().entity_mut(entity).remove::<Mode>();

    assert!(!has::<ModeIdle>(&app, entity)?);
    assert!(!has::<ModeRun>(&app, entity)?);
    Ok(())
}

#[test]
fn variant_change_removes_the_old_marker_and_adds_the_new_one() -> Result<()> {
    let mut app = app_with::<Mode>();
    Transitions::observe::<ModeIdle>(&mut app);
    Transitions::observe::<ModeRun>(&mut app);

    let entity = app.world_mut().spawn(Mode::Idle).id();
    app.world_mut().entity_mut(entity).insert(Mode::Run);

    assert_eq!(
        Transitions::of::<ModeIdle>(&app)?,
        Transitions::new(1, 1, 1, 1)
    );
    assert_eq!(
        Transitions::of::<ModeRun>(&app)?,
        Transitions::new(1, 1, 0, 0)
    );
    Ok(())
}

#[test]
fn reinserting_the_same_variant_leaves_the_marker_alone() -> Result<()> {
    let mut app = app_with::<Mode>();
    Transitions::observe::<ModeIdle>(&mut app);

    let entity = app.world_mut().spawn(Mode::Idle).id();
    app.world_mut().entity_mut(entity).insert(Mode::Idle);
    app.update();

    assert_eq!(
        Transitions::of::<ModeIdle>(&app)?,
        Transitions::new(1, 1, 0, 0)
    );
    Ok(())
}

#[test]
fn immutable_enums_get_no_per_frame_system() {
    let app = app_with::<Mode>();
    let systems = systems_in(&app, PreUpdate);
    assert_eq!(systems, 0);
}

#[test]
fn in_place_mutation_is_synced_by_the_changed_system() -> Result<()> {
    let mut app = app_with::<Gait>();
    let entity = app.world_mut().spawn(Gait::Walk).id();

    if let Some(mut gait) = app.world_mut().get_mut::<Gait>(entity) {
        *gait = Gait::Sprint;
    }
    assert!(
        has::<GaitWalk>(&app, entity)?,
        "nothing observes Mut<T> edits"
    );

    app.update();
    assert!(!has::<GaitWalk>(&app, entity)?);
    assert!(has::<GaitSprint>(&app, entity)?);
    Ok(())
}

#[test]
fn in_place_mutation_to_the_same_variant_leaves_the_marker_alone() -> Result<()> {
    let mut app = app_with::<Gait>();
    Transitions::observe::<GaitWalk>(&mut app);

    let entity = app.world_mut().spawn(Gait::Walk).id();
    if let Some(mut gait) = app.world_mut().get_mut::<Gait>(entity) {
        *gait = Gait::Walk;
    }
    app.update();

    assert_eq!(
        Transitions::of::<GaitWalk>(&app)?,
        Transitions::new(1, 1, 0, 0)
    );
    Ok(())
}

#[test]
fn systems_after_the_set_see_same_frame_mutations() -> Result<()> {
    #[derive(Resource, Default)]
    struct Sprinters(usize);

    let mut app = app_with::<Gait>();
    app.init_resource::<Sprinters>().add_systems(
        PreUpdate,
        (|q: Query<(), With<GaitSprint>>, mut seen: ResMut<Sprinters>| seen.0 = q.count())
            .after(EnumFilterSystems),
    );

    let entity = app.world_mut().spawn(Gait::Walk).id();
    app.update();
    assert_eq!(app.world().resource::<Sprinters>().0, 0);

    if let Some(mut gait) = app.world_mut().get_mut::<Gait>(entity) {
        *gait = Gait::Sprint;
    }
    app.update();
    assert_eq!(app.world().resource::<Sprinters>().0, 1);
    assert!(has::<GaitSprint>(&app, entity)?);
    Ok(())
}

#[test]
fn in_schedule_moves_the_changed_system() -> Result<()> {
    let mut app = App::new();
    app.add_plugins(EnumFilterPlugin::<Gait>::in_schedule(Update));

    assert_eq!(systems_in(&app, PreUpdate), 0);
    assert_eq!(systems_in(&app, Update), 1);

    let entity = app.world_mut().spawn(Gait::Walk).id();
    if let Some(mut gait) = app.world_mut().get_mut::<Gait>(entity) {
        *gait = Gait::Sprint;
    }
    app.update();
    assert!(has::<GaitSprint>(&app, entity)?);
    Ok(())
}

#[test]
fn inserts_from_commands_are_synced_when_commands_apply() -> Result<()> {
    #[derive(Component)]
    struct Spawned;

    let mut app = app_with::<Mode>();
    app.add_systems(Update, |mut commands: Commands| {
        commands.spawn((Spawned, Mode::Run));
    });
    app.update();

    let mut query = app
        .world_mut()
        .query_filtered::<(), (With<Spawned>, With<ModeRun>)>();
    assert_eq!(query.iter(app.world()).count(), 1);
    Ok(())
}

#[test]
fn entities_spawned_before_the_plugin_are_backfilled() -> Result<()> {
    let mut app = App::new();
    let entity = app.world_mut().spawn(Mode::Run).id();
    app.add_plugins(EnumFilterPlugin::<Mode>::default());

    assert!(has::<ModeRun>(&app, entity)?);
    Ok(())
}

#[test]
fn despawning_an_entity_with_the_enum_is_quiet() {
    let mut app = app_with::<Mode>();
    let entity = app.world_mut().spawn(Mode::Idle).id();
    app.world_mut().despawn(entity);
    app.update();

    assert!(app.world().get_entity(entity).is_err());
}

#[test]
fn an_observer_despawning_mid_transition_does_not_panic() {
    let mut app = app_with::<Mode>();
    app.add_observer(|remove: On<Remove<ModeIdle>>, mut commands: Commands| {
        commands.entity(remove.entity).despawn();
    });

    let entity = app.world_mut().spawn(Mode::Idle).id();
    app.world_mut().entity_mut(entity).insert(Mode::Run);

    assert!(app.world().get_entity(entity).is_err());
}

#[derive(Component, EnumFilter, Debug)]
#[component(immutable)]
enum Shape {
    Point,
    Circle(f32),
    Rect { w: f32 },
}

impl Shape {
    fn size(&self) -> f32 {
        match self {
            Self::Point => 0.0,
            Self::Circle(r) => *r,
            Self::Rect { w } => *w,
        }
    }
}

#[test]
fn tuple_and_struct_variants_get_markers() -> Result<()> {
    let mut app = app_with::<Shape>();
    let entity = app.world_mut().spawn(Shape::Circle(1.0)).id();
    assert!(has::<ShapeCircle>(&app, entity)?);

    app.world_mut()
        .entity_mut(entity)
        .insert(Shape::Rect { w: 2.0 });
    assert!(!has::<ShapeCircle>(&app, entity)?);
    assert!(has::<ShapeRect>(&app, entity)?);

    app.world_mut().entity_mut(entity).insert(Shape::Point);
    assert!(has::<ShapePoint>(&app, entity)?);
    assert!(!has::<ShapeRect>(&app, entity)?);

    let size = app.world().get::<Shape>(entity).map(Shape::size);
    assert_eq!(size, Some(0.0));
    Ok(())
}

#[derive(Component, EnumFilter, Debug, Clone, Copy)]
#[component(immutable)]
enum Wide {
    V00,
    V01,
    V02,
    V03,
    V04,
    V05,
    V06,
    V07,
    V08,
    V09,
    V10,
    V11,
    V12,
    V13,
    V14,
    V15,
    V16,
}

#[test]
fn enums_wider_than_the_bundle_tuple_limit_work() -> Result<()> {
    let mut app = app_with::<Wide>();
    let entity = app.world_mut().spawn(Wide::V16).id();
    assert!(has::<WideV16>(&app, entity)?);

    app.world_mut().entity_mut(entity).insert(Wide::V00);
    assert!(has::<WideV00>(&app, entity)?);
    assert!(!has::<WideV16>(&app, entity)?);

    app.world_mut().entity_mut(entity).remove::<Wide>();
    assert!(!has::<WideV00>(&app, entity)?);
    Ok(())
}

#[test]
fn variant_indexes_follow_declaration_order() {
    use Wide::*;
    let all = [
        V00, V01, V02, V03, V04, V05, V06, V07, V08, V09, V10, V11, V12, V13, V14, V15, V16,
    ];
    assert_eq!(all.len(), Wide::VARIANT_COUNT);
    for (index, variant) in all.iter().enumerate() {
        assert_eq!(variant.variant_index(), index);
    }
}

#[test]
fn markers_bundle_strips_every_marker() -> Result<()> {
    let mut app = App::new();
    let entity = app.world_mut().spawn((ModeIdle, ModeRun)).id();
    app.world_mut()
        .entity_mut(entity)
        .remove::<<Mode as EnumFilter>::Markers>();

    assert!(!has::<ModeIdle>(&app, entity)?);
    assert!(!has::<ModeRun>(&app, entity)?);
    Ok(())
}

#[derive(Component, EnumFilter)]
#[component(immutable)]
#[enum_filter(storage = "Table")]
enum Tabled {
    On,
    Off,
}

#[test]
fn markers_are_immutable_sparse_set_unless_asked_otherwise() -> Result<()> {
    assert_eq!(ModeIdle::STORAGE_TYPE, StorageType::SparseSet);
    assert_eq!(TabledOn::STORAGE_TYPE, StorageType::Table);
    const { assert!(!<ModeIdle as Component>::Mutability::MUTABLE) };

    let mut app = app_with::<Tabled>();
    let entity = app.world_mut().spawn(Tabled::On).id();
    app.world_mut().entity_mut(entity).insert(Tabled::Off);
    assert!(has::<TabledOff>(&app, entity)?);
    assert!(!has::<TabledOn>(&app, entity)?);
    Ok(())
}
