use core::borrow::Borrow;

use bevy_ecs::bundle::Bundle;
use bevy_ecs::component::Component;
use bevy_ecs::world::EntityWorldMut;

/// An enum with one marker component per variant.
///
/// Implement it with [`#[derive(EnumFilter)]`](derive@crate::EnumFilter),
/// which also generates the markers. A manual impl is not supported, the
/// hidden methods are a contract with the generated code.
///
/// The trait describes the markers. Keeping them on entities is the job of
/// [`EnumFilterPlugin`](crate::EnumFilterPlugin) (enum is the component) or
/// [`EnumFilterCollectionPlugin`](crate::EnumFilterCollectionPlugin) (a
/// component holds many values of the enum).
pub trait EnumFilter {
    /// Every marker generated for this enum, in variant declaration order.
    ///
    /// Handy for stripping all of them at once with
    /// `entity.remove::<<MyEnum as EnumFilter>::Markers>()`.
    type Markers: Bundle;

    /// Number of variants, and therefore of markers.
    const VARIANT_COUNT: usize;

    /// Position of this value's variant in declaration order, always below
    /// [`VARIANT_COUNT`](Self::VARIANT_COUNT).
    fn variant_index(&self) -> usize;

    /// Calls `visitor` with the marker type of the variant at `index`, or
    /// returns `None` when `index` is out of range.
    #[doc(hidden)]
    fn visit_marker<V: MarkerVisitor>(index: usize, visitor: V) -> Option<V::Output>;

    /// Brings markers and extracted payloads on `entity` in line with its
    /// `C` component, or strips them when `C` is absent.
    ///
    /// Generic over `C` rather than requiring `Self: Component`, so enums
    /// that only ever live inside a collection still get a valid impl.
    #[doc(hidden)]
    fn sync_markers<C>(entity: &mut EntityWorldMut<'_>)
    where
        C: Component + Borrow<Self>;
}

/// Type-level callback over a single marker type, see
/// [`EnumFilter::visit_marker`]. Lets generic code act on "the marker of
/// variant `n`" without the derive generating one method per operation.
#[doc(hidden)]
pub trait MarkerVisitor {
    type Output;

    fn visit<M: Component + Default>(self) -> Self::Output;
}
