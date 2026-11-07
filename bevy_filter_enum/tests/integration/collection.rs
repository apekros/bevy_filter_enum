use anyhow::Result;
use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_filter_enum::prelude::*;

use crate::{Transitions, has, systems_in};

#[derive(EnumFilter, Debug, Clone, Copy, PartialEq, Eq)]
enum Fruit {
    Apple,
    Banana,
    Orange,
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    fruit: Fruit,
}

/// Mutable, and the enum is reached through a field.
#[derive(Component, Debug, Clone)]
struct Basket(Vec<Slot>);

impl EnumFilterCollection for Basket {
    type Filter = Fruit;

    fn variants(&self) -> impl Iterator<Item = &Fruit> {
        self.0.iter().map(|slot| &slot.fruit)
    }
}

fn basket(fruits: &[Fruit]) -> Basket {
    Basket(fruits.iter().map(|&fruit| Slot { fruit }).collect())
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(EnumFilterCollectionPlugin::<Basket>::default());
    app
}

#[test]
fn every_present_variant_gets_a_marker() -> Result<()> {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn(basket(&[Fruit::Apple, Fruit::Banana, Fruit::Apple]))
        .id();

    assert!(has::<FruitApple>(&app, entity)?);
    assert!(has::<FruitBanana>(&app, entity)?);
    assert!(!has::<FruitOrange>(&app, entity)?);
    Ok(())
}

#[test]
fn markers_kept_across_an_update_are_not_touched() -> Result<()> {
    let mut app = app();
    Transitions::observe::<FruitApple>(&mut app);

    let entity = app.world_mut().spawn(basket(&[Fruit::Apple])).id();
    app.world_mut()
        .entity_mut(entity)
        .insert(basket(&[Fruit::Apple, Fruit::Banana]));

    assert_eq!(
        Transitions::of::<FruitApple>(&app)?,
        Transitions::new(1, 1, 0, 0)
    );
    assert!(has::<FruitBanana>(&app, entity)?);
    Ok(())
}

#[test]
fn markers_follow_replacement() -> Result<()> {
    let mut app = app();
    let entity = app.world_mut().spawn(basket(&[Fruit::Apple])).id();
    app.world_mut()
        .entity_mut(entity)
        .insert(basket(&[Fruit::Orange]));

    assert!(!has::<FruitApple>(&app, entity)?);
    assert!(!has::<FruitBanana>(&app, entity)?);
    assert!(has::<FruitOrange>(&app, entity)?);
    Ok(())
}

#[test]
fn markers_follow_in_place_mutation() -> Result<()> {
    let mut app = app();
    let entity = app.world_mut().spawn(basket(&[Fruit::Apple])).id();
    if let Some(mut basket) = app.world_mut().get_mut::<Basket>(entity) {
        basket.0.clear();
        basket.0.push(Slot {
            fruit: Fruit::Banana,
        });
    }
    app.update();

    assert!(!has::<FruitApple>(&app, entity)?);
    assert!(has::<FruitBanana>(&app, entity)?);
    Ok(())
}

#[test]
fn removing_the_collection_removes_every_marker() -> Result<()> {
    let mut app = app();
    let entity = app
        .world_mut()
        .spawn(basket(&[Fruit::Apple, Fruit::Orange]))
        .id();
    app.world_mut().entity_mut(entity).remove::<Basket>();

    assert!(!has::<FruitApple>(&app, entity)?);
    assert!(!has::<FruitOrange>(&app, entity)?);
    Ok(())
}

/// Immutable, holding the enum values directly.
#[derive(Component)]
#[component(immutable)]
struct Tags(Vec<Fruit>);

impl EnumFilterCollection for Tags {
    type Filter = Fruit;

    fn variants(&self) -> impl Iterator<Item = &Fruit> {
        self.0.iter()
    }
}

#[test]
fn immutable_collections_sync_without_a_system() -> Result<()> {
    let mut app = App::new();
    app.add_plugins(EnumFilterCollectionPlugin::<Tags>::default());
    let entity = app.world_mut().spawn(Tags(vec![Fruit::Orange])).id();

    assert!(has::<FruitOrange>(&app, entity)?);
    assert_eq!(systems_in(&app, bevy_app::PreUpdate), 0);
    Ok(())
}
