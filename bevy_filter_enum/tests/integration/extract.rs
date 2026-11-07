use anyhow::Result;
use bevy_app::App;
use bevy_ecs::lifecycle::Add;
use bevy_ecs::observer::On;
use bevy_ecs::prelude::{Commands, Component, Query, ResMut, Resource};
use bevy_filter_enum::prelude::*;

use crate::{Transitions, get, has};

#[derive(Component, Debug, Clone, PartialEq, Eq)]
struct Progress {
    percent: u8,
}

#[derive(Component, EnumFilter, Debug, Clone, PartialEq, Eq)]
enum Download {
    Queued,
    #[enum_filter(extract)]
    Active(Progress),
    #[enum_filter(extract)]
    Paused(Progress),
    Finished,
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(EnumFilterPlugin::<Download>::default());
    app
}

fn active(percent: u8) -> Download {
    Download::Active(Progress { percent })
}

#[test]
fn payload_is_mirrored_on_insert() -> Result<()> {
    let mut app = app();
    let entity = app.world_mut().spawn(active(40)).id();

    assert!(has::<DownloadActive>(&app, entity)?);
    assert_eq!(get(&app, entity)?, Some(Progress { percent: 40 }));
    Ok(())
}

#[test]
fn payload_update_within_a_variant_is_a_replace() -> Result<()> {
    let mut app = app();
    Transitions::observe::<Progress>(&mut app);

    let entity = app.world_mut().spawn(active(40)).id();
    app.world_mut().entity_mut(entity).insert(active(80));

    assert_eq!(
        Transitions::of::<Progress>(&app)?,
        Transitions::new(1, 2, 1, 0)
    );
    assert_eq!(get(&app, entity)?, Some(Progress { percent: 80 }));
    Ok(())
}

#[test]
fn payload_shared_by_two_variants_is_replaced_not_readded() -> Result<()> {
    let mut app = app();
    Transitions::observe::<Progress>(&mut app);

    let entity = app.world_mut().spawn(active(40)).id();
    app.world_mut()
        .entity_mut(entity)
        .insert(Download::Paused(Progress { percent: 40 }));

    assert!(!has::<DownloadActive>(&app, entity)?);
    assert!(has::<DownloadPaused>(&app, entity)?);
    assert_eq!(
        Transitions::of::<Progress>(&app)?,
        Transitions::new(1, 2, 1, 0)
    );
    Ok(())
}

#[test]
fn payload_goes_away_with_its_variant() -> Result<()> {
    let mut app = app();
    let entity = app.world_mut().spawn(active(40)).id();
    app.world_mut()
        .entity_mut(entity)
        .insert(Download::Finished);

    assert!(has::<DownloadFinished>(&app, entity)?);
    assert!(!has::<Progress>(&app, entity)?);
    Ok(())
}

#[test]
fn payload_goes_away_with_the_enum() -> Result<()> {
    let mut app = app();
    let entity = app.world_mut().spawn(active(40)).id();
    app.world_mut().entity_mut(entity).remove::<Download>();

    assert!(!has::<Progress>(&app, entity)?);
    assert!(!has::<DownloadActive>(&app, entity)?);
    Ok(())
}

#[test]
fn payload_follows_in_place_mutation() -> Result<()> {
    let mut app = app();
    let entity = app.world_mut().spawn(active(40)).id();
    if let Some(mut download) = app.world_mut().get_mut::<Download>(entity) {
        *download = active(80);
    }
    app.update();

    assert_eq!(get(&app, entity)?, Some(Progress { percent: 80 }));
    Ok(())
}

#[test]
fn marker_observers_already_see_the_payload() -> Result<()> {
    #[derive(Resource, Default)]
    struct SeenOnAdd(Option<u8>);

    let mut app = app();
    app.init_resource::<SeenOnAdd>().add_observer(
        |add: On<Add<DownloadActive>>, progress: Query<&Progress>, mut seen: ResMut<SeenOnAdd>| {
            seen.0 = progress.get(add.entity).ok().map(|p| p.percent);
        },
    );

    app.world_mut().spawn(Download::Queued).insert(active(10));

    assert_eq!(app.world().resource::<SeenOnAdd>().0, Some(10));
    Ok(())
}

#[test]
fn an_observer_despawning_on_marker_add_does_not_panic() {
    let mut app = app();
    app.add_observer(|add: On<Add<DownloadActive>>, mut commands: Commands| {
        commands.entity(add.entity).despawn();
    });

    let entity = app.world_mut().spawn(Download::Queued).id();
    app.world_mut().entity_mut(entity).insert(active(40));

    assert!(app.world().get_entity(entity).is_err());
}

#[derive(Component, Debug, Clone, PartialEq, Eq)]
struct Route {
    distance: u32,
}

#[derive(Component, EnumFilter, Debug, Clone, PartialEq, Eq)]
#[component(immutable)]
enum Step {
    Wait,
    #[enum_filter(extract)]
    Navigate(Route),
}

#[derive(Component, EnumFilter, Debug, Clone, PartialEq, Eq)]
#[component(immutable)]
enum Task {
    Created,
    #[enum_filter(extract)]
    Executing(Step),
}

fn nested_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        EnumFilterPlugin::<Task>::default(),
        EnumFilterPlugin::<Step>::default(),
    ));
    app
}

#[test]
fn extraction_cascades_through_nested_filters() -> Result<()> {
    let mut app = nested_app();
    let route = Route { distance: 12 };
    let entity = app
        .world_mut()
        .spawn(Task::Executing(Step::Navigate(route.clone())))
        .id();

    assert!(has::<TaskExecuting>(&app, entity)?);
    assert!(has::<StepNavigate>(&app, entity)?);
    assert_eq!(get(&app, entity)?, Some(route));
    Ok(())
}

#[test]
fn nested_cascade_unwinds_when_the_outer_variant_changes() -> Result<()> {
    let mut app = nested_app();
    let entity = app
        .world_mut()
        .spawn(Task::Executing(Step::Navigate(Route { distance: 12 })))
        .id();
    app.world_mut()
        .entity_mut(entity)
        .insert(Task::Executing(Step::Wait));
    assert!(has::<StepWait>(&app, entity)?);
    assert!(!has::<StepNavigate>(&app, entity)?);
    assert!(!has::<Route>(&app, entity)?);

    app.world_mut().entity_mut(entity).insert(Task::Created);
    assert!(has::<TaskCreated>(&app, entity)?);
    assert!(!has::<StepWait>(&app, entity)?);
    assert!(!has::<Step>(&app, entity)?);
    assert!(!has::<StepNavigate>(&app, entity)?);
    assert!(!has::<Route>(&app, entity)?);
    Ok(())
}
