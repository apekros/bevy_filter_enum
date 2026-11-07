//! Query Bevy entities by enum variant.
//!
//! `With<T>` can only see component types, not values, so filtering on
//! `Mode::Run` normally means fetching every `Mode` and matching it. This
//! crate gives each variant a zero-sized marker component and keeps exactly
//! the active one on the entity, so the filter happens at archetype level.
//!
//! ```
//! use bevy_app::App;
//! use bevy_ecs::prelude::*;
//! use bevy_filter_enum::prelude::*;
//!
//! #[derive(Component, EnumFilter)]
//! #[component(immutable)]
//! enum Mode {
//!     Idle,
//!     Run { speed: f32 },
//! }
//!
//! // `ModeIdle` and `ModeRun` are generated next to `Mode`.
//! fn runners(query: Query<Entity, With<ModeRun>>) {
//!     for entity in &query {
//!         // ...
//!     }
//! }
//!
//! let mut app = App::new();
//! app.add_plugins(EnumFilterPlugin::<Mode>::default());
//!
//! let entity = app.world_mut().spawn(Mode::Idle).id();
//! assert!(app.world().entity(entity).contains::<ModeIdle>());
//!
//! app.world_mut().entity_mut(entity).insert(Mode::Run { speed: 2.0 });
//! assert!(app.world().entity(entity).contains::<ModeRun>());
//! assert!(!app.world().entity(entity).contains::<ModeIdle>());
//! ```
//!
//! # Keeping markers in sync
//!
//! [`EnumFilterPlugin`] observes inserts and removals of the enum, so markers
//! are correct as soon as the change is applied.
//!
//! Mutable components can also change in place through `Mut<T>`, which no
//! observer sees. For those the plugin adds a `Changed<T>` system to
//! [`EnumFilterSystems`] in `PreUpdate` (see
//! [`EnumFilterPlugin::in_schedule`]). Prefer `#[component(immutable)]`: you
//! get immediate markers for every change and no per-frame system.
//!
//! # Attributes
//!
//! - `#[enum_filter(storage = "Table")]` on the enum stores the markers in
//!   tables. The default is `"SparseSet"`, because markers churn on every
//!   transition and sparse-set components are added and removed without
//!   moving the entity's table data. Pick `"Table"` for enums that rarely
//!   change but are filtered in very hot queries.
//! - `#[enum_filter(extract)]` on a single-field tuple variant mirrors its
//!   payload as a component while the variant is active, so systems can
//!   query `&Payload` directly. The payload must be `Component + Clone`. The
//!   enum stays the source of truth, edits to the mirrored copy are
//!   overwritten on the next sync.
//!
//! ```
//! # use bevy_ecs::prelude::*;
//! # use bevy_filter_enum::prelude::*;
//! #[derive(Component, Clone)]
//! struct Progress(u8);
//!
//! #[derive(Component, EnumFilter)]
//! #[component(immutable)]
//! #[enum_filter(storage = "Table")]
//! enum Download {
//!     Queued,
//!     #[enum_filter(extract)]
//!     Active(Progress),
//!     Done,
//! }
//!
//! fn show_progress(query: Query<&Progress, With<DownloadActive>>) {
//!     for progress in &query {
//!         // ...
//!     }
//! }
//! ```
//!
//! # Collections
//!
//! When one component holds many enum values, implement
//! [`EnumFilterCollection`] and add [`EnumFilterCollectionPlugin`]. The entity
//! then carries the marker of every variant present in the collection.

mod collection;
mod filter;
mod plugin;
mod sync;

pub use bevy_filter_enum_derive::EnumFilter;

pub use crate::collection::EnumFilterCollection;
pub use crate::filter::EnumFilter;
pub use crate::plugin::{EnumFilterCollectionPlugin, EnumFilterPlugin, EnumFilterSystems};

/// `use bevy_filter_enum::prelude::*;` for everything an app needs.
pub mod prelude {
    pub use crate::{
        EnumFilter, EnumFilterCollection, EnumFilterCollectionPlugin, EnumFilterPlugin,
        EnumFilterSystems,
    };
}

/// Paths for the derive output. Not public API.
#[doc(hidden)]
pub mod __private {
    pub use bevy_ecs;

    pub use crate::filter::MarkerVisitor;
    pub use crate::sync::{assert_payload, clear, remove, set_variant, set_variant_with_payload};
}
