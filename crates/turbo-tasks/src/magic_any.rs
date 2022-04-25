use core::fmt;
use mopa::mopafy;
use std::{
    any::{type_name, TypeId},
    cmp::Ordering,
    fmt::Debug,
    hash::{Hash, Hasher},
    ops::DerefMut,
};

pub trait MagicAny: mopa::Any + Send + Sync {
    fn magic_debug(&self, f: &mut fmt::Formatter) -> fmt::Result;

    fn magic_eq(&self, other: &dyn MagicAny) -> bool;

    fn magic_hash(&self, hasher: &mut dyn Hasher);

    fn magic_cmp(&self, other: &dyn MagicAny) -> Ordering;
}
mopafy!(MagicAny);

impl<T: Debug + Eq + Ord + Hash + Send + Sync + 'static> MagicAny for T {
    fn magic_debug(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut d = f.debug_tuple("MagicAny");
        d.field(&TypeId::of::<Self>());

        #[cfg(debug_assertions)]
        d.field(&type_name::<Self>());

        d.field(&(self as &Self));
        d.finish()
    }

    fn magic_eq(&self, other: &dyn MagicAny) -> bool {
        match other.downcast_ref::<Self>() {
            None => false,
            Some(other) => self == other,
        }
    }

    fn magic_hash(&self, hasher: &mut dyn Hasher) {
        Hash::hash(&(TypeId::of::<Self>(), self), &mut HasherMut(hasher))
    }

    fn magic_cmp(&self, other: &dyn MagicAny) -> Ordering {
        match other.downcast_ref::<Self>() {
            None => Ord::cmp(&TypeId::of::<Self>(), &other.type_id()),
            Some(other) => self.cmp(other),
        }
    }
}

impl fmt::Debug for dyn MagicAny {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.magic_debug(f)
    }
}

impl PartialEq for dyn MagicAny {
    fn eq(&self, other: &Self) -> bool {
        self.magic_eq(other)
    }
}

impl Eq for dyn MagicAny {}

impl PartialOrd for dyn MagicAny {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.magic_cmp(other))
    }
}

impl Ord for dyn MagicAny {
    fn cmp(&self, other: &Self) -> Ordering {
        self.magic_cmp(other)
    }
}

impl Hash for dyn MagicAny {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.magic_hash(hasher)
    }
}

pub struct HasherMut<H: ?Sized>(H);

impl<H: DerefMut + ?Sized> Hasher for HasherMut<H>
where
    H::Target: Hasher,
{
    fn finish(&self) -> u64 {
        self.0.finish()
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes)
    }
}
