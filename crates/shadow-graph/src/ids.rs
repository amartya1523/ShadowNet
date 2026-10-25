//! Dense integer ids. Everything in the simulator is stored in `Vec`s indexed
//! by these, so they are deliberately small and `Copy`.

use std::fmt;

macro_rules! dense_id {
    ($(#[$meta:meta])* $name:ident, $repr:ty, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(pub $repr);

        impl $name {
            #[inline]
            pub const fn idx(self) -> usize {
                self.0 as usize
            }

            #[inline]
            pub fn from_idx(i: usize) -> Self {
                debug_assert!(i <= <$repr>::MAX as usize);
                Self(i as $repr)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(self, f)
            }
        }
    };
}

dense_id!(RouterId, u32, "r");
dense_id!(LinkId, u32, "l");
dense_id!(
    /// Index into `Topology::ases`. Not the ASN; see `AutonomousSystem::asn`.
    AsId,
    u16,
    "as"
);
dense_id!(HostId, u32, "h");
dense_id!(CityId, u16, "c");
dense_id!(ServiceId, u16, "svc");
