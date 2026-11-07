use bevy_ecs::component::Component;
use bevy_ecs::world::EntityWorldMut;

use crate::EnumFilter;
use crate::filter::MarkerVisitor;

/// A component holding any number of [`EnumFilter`] values, whose entity
/// should carry the marker of every variant present.
///
/// ```
/// use bevy_ecs::prelude::*;
/// use bevy_filter_enum::prelude::*;
///
/// #[derive(EnumFilter)]
/// enum Fruit {
///     Apple,
///     Banana,
/// }
///
/// struct Slot {
///     fruit: Fruit,
///     count: u32,
/// }
///
/// #[derive(Component)]
/// struct Basket(Vec<Slot>);
///
/// impl EnumFilterCollection for Basket {
///     type Filter = Fruit;
///
///     fn variants(&self) -> impl Iterator<Item = &Fruit> {
///         self.0.iter().map(|slot| &slot.fruit)
///     }
/// }
///
/// // Then register it with
/// // `app.add_plugins(EnumFilterCollectionPlugin::<Basket>::default())`,
/// // and query baskets holding apples with `With<FruitApple>`.
/// ```
pub trait EnumFilterCollection: Component {
    /// The enum whose markers this collection drives.
    type Filter: EnumFilter;

    /// Every value currently in the collection. Duplicates are fine.
    fn variants(&self) -> impl Iterator<Item = &Self::Filter>;
}

/// Adds the marker of every variant in `C` and removes the rest. Markers that
/// are already right are not touched.
pub(crate) fn sync_collection<C: EnumFilterCollection>(entity: &mut EntityWorldMut<'_>) {
    let mut active = vec![false; C::Filter::VARIANT_COUNT];
    if let Some(collection) = entity.get::<C>() {
        for value in collection.variants() {
            if let Some(slot) = active.get_mut(value.variant_index()) {
                *slot = true;
            }
        }
    }
    for (index, active) in active.into_iter().enumerate() {
        if entity.is_despawned() {
            return;
        }
        C::Filter::visit_marker(index, ToggleMarker { entity, active });
    }
}

/// Adds or removes one marker so its presence matches `active`.
struct ToggleMarker<'a, 'w> {
    entity: &'a mut EntityWorldMut<'w>,
    active: bool,
}

impl MarkerVisitor for ToggleMarker<'_, '_> {
    type Output = ();

    fn visit<M: Component + Default>(self) {
        match (self.active, self.entity.contains::<M>()) {
            (true, false) => {
                self.entity.insert(M::default());
            }
            (false, true) => {
                self.entity.remove::<M>();
            }
            _ => {}
        }
    }
}
