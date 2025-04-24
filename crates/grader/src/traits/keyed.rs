use std::borrow::Borrow;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub trait Keyed {
	type Key: Eq + Hash;
	fn key(&self) -> Self::Key;
}


#[derive(Clone, Debug)]
pub struct ArcKey<T: Keyed + ?Sized>(pub Arc<T>);

impl<T: Keyed + ?Sized> PartialEq for ArcKey<T> {
	fn eq(&self, other: &Self) -> bool {
		self.0.key() == other.0.key()
	}
}
impl<T: Keyed + ?Sized> Eq for ArcKey<T> {}

impl<T: Keyed + ?Sized> Hash for ArcKey<T> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.0.key().hash(state);
	}
}

impl<T: Keyed + Sized> From<T> for ArcKey<T> {
	fn from(arc: T) -> Self {
		ArcKey(Arc::new(arc))
	}
}

impl<T: Keyed + ?Sized> Borrow<T> for ArcKey<T> {
	fn borrow(&self) -> &T {
		&self.0
	}
}