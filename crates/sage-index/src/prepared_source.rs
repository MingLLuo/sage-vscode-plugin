//! Per-thread memo of whole-file analysis structures for one source text.
//!
//! Reference search resolves every occurrence of a name in a file (thousands of ranges for
//! common Sage names). Many resolution helpers take only `&str` and used to rebuild the same
//! whole-file structures for each range. Inside a [`PreparedSourceScope`] those helpers share
//! one instance per file instead.
//!
//! The scope borrows the source for its whole lifetime, so its address and length identify the
//! text unambiguously: no other string can occupy that memory while the scope is alive.

use crate::model::SymbolRecord;
use crate::query_support::LexicalScopeMap;
use crate::syntax_support::CodeMap;
use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

#[derive(Default)]
struct Prepared {
    key: (usize, usize),
    code_map: Option<Rc<CodeMap>>,
    lexical_scopes: Option<Rc<LexicalScopeMap>>,
    import_resolutions: HashMap<(String, String, String), Option<SymbolRecord>>,
    derived: HashMap<TypeId, Rc<dyn Any>>,
    symbol_candidates: HashMap<String, Rc<Vec<SymbolRecord>>>,
}

thread_local! {
    static PREPARED: RefCell<Option<Prepared>> = const { RefCell::new(None) };
}

fn key(source: &str) -> (usize, usize) {
    (source.as_ptr() as usize, source.len())
}

/// Shares whole-file analysis structures for `source` until dropped. Scopes nest: an inner
/// scope for another text is restored to the outer one when it ends.
pub(crate) struct PreparedSourceScope<'a> {
    previous: Option<Prepared>,
    _source: PhantomData<&'a str>,
}

pub(crate) fn prepare_source(source: &str) -> PreparedSourceScope<'_> {
    let previous = PREPARED.with(|prepared| {
        prepared.borrow_mut().replace(Prepared {
            key: key(source),
            ..Prepared::default()
        })
    });
    PreparedSourceScope {
        previous,
        _source: PhantomData,
    }
}

impl Drop for PreparedSourceScope<'_> {
    fn drop(&mut self) {
        let previous = self.previous.take();
        PREPARED.with(|prepared| *prepared.borrow_mut() = previous);
    }
}

fn shared<T>(
    source: &str,
    slot: fn(&mut Prepared) -> &mut Option<Rc<T>>,
    build: impl FnOnce() -> T,
) -> Rc<T> {
    let cached = PREPARED.with(|prepared| {
        let mut prepared = prepared.borrow_mut();
        let prepared = prepared
            .as_mut()
            .filter(|prepared| prepared.key == key(source))?;
        slot(prepared).clone()
    });
    if let Some(cached) = cached {
        return cached;
    }
    // Build without holding the borrow: construction may itself ask for shared structures.
    let built = Rc::new(build());
    PREPARED.with(|prepared| {
        if let Some(prepared) = prepared
            .borrow_mut()
            .as_mut()
            .filter(|prepared| prepared.key == key(source))
        {
            *slot(prepared) = Some(built.clone());
        }
    });
    built
}

impl CodeMap {
    /// Like [`CodeMap::new`], but reuses the instance of an active [`PreparedSourceScope`].
    pub(crate) fn shared(source: &str) -> Rc<CodeMap> {
        shared(
            source,
            |prepared| &mut prepared.code_map,
            || CodeMap::new(source),
        )
    }
}

impl LexicalScopeMap {
    /// Like [`LexicalScopeMap::new`], but reuses the instance of an active scope.
    pub(crate) fn shared(source: &str) -> Rc<LexicalScopeMap> {
        shared(
            source,
            |prepared| &mut prepared.lexical_scopes,
            || LexicalScopeMap::new(source),
        )
    }
}

/// Memoizes import resolution while a scope is active. Every occurrence of an imported name in
/// a file resolves the same import record; resolving it reads and parses the target module, so
/// doing that once per file instead of once per occurrence dominates reference-search cost.
/// Without an active scope `resolve` simply runs.
pub(crate) fn memoized_import_resolution(
    symbol: &SymbolRecord,
    resolve: impl FnOnce() -> Option<SymbolRecord>,
) -> Option<SymbolRecord> {
    let key = (
        symbol.module.clone(),
        symbol.name.clone(),
        symbol.import_from.clone().unwrap_or_default(),
    );
    let cached = PREPARED.with(|prepared| {
        prepared
            .borrow()
            .as_ref()
            .map(|prepared| prepared.import_resolutions.get(&key).cloned())
    });
    match cached {
        None => resolve(),
        Some(Some(result)) => result,
        Some(None) => {
            let result = resolve();
            PREPARED.with(|prepared| {
                if let Some(prepared) = prepared.borrow_mut().as_mut() {
                    prepared.import_resolutions.insert(key, result.clone());
                }
            });
            result
        }
    }
}

/// Returns a whole-file value derived from `source`, built at most once per active scope.
/// Returns `None` when no scope for `source` is active, so callers keep their direct path.
pub(crate) fn shared_derived<T: 'static>(source: &str, build: impl FnOnce() -> T) -> Option<Rc<T>> {
    let state = PREPARED.with(|prepared| {
        let prepared = prepared.borrow();
        let prepared = prepared
            .as_ref()
            .filter(|prepared| prepared.key == key(source))?;
        Some(prepared.derived.get(&TypeId::of::<T>()).cloned())
    })?;
    if let Some(value) = state {
        return value.downcast::<T>().ok();
    }
    let built = Rc::new(build());
    PREPARED.with(|prepared| {
        if let Some(prepared) = prepared
            .borrow_mut()
            .as_mut()
            .filter(|prepared| prepared.key == key(source))
        {
            prepared
                .derived
                .insert(TypeId::of::<T>(), built.clone() as Rc<dyn Any>);
        }
    });
    Some(built)
}

/// Memoizes global symbol candidates by lowercase name while a scope is active; resolving each
/// occurrence of `matrix` asked the index for the same candidate list and deduplicated it again.
pub(crate) fn memoized_symbol_candidates(
    key: &str,
    load: impl FnOnce() -> Vec<SymbolRecord>,
) -> Vec<SymbolRecord> {
    let cached = PREPARED.with(|prepared| {
        prepared
            .borrow()
            .as_ref()
            .map(|prepared| prepared.symbol_candidates.get(key).cloned())
    });
    match cached {
        None => load(),
        Some(Some(symbols)) => symbols.as_ref().clone(),
        Some(None) => {
            let symbols = Rc::new(load());
            PREPARED.with(|prepared| {
                if let Some(prepared) = prepared.borrow_mut().as_mut() {
                    prepared
                        .symbol_candidates
                        .insert(key.to_string(), symbols.clone());
                }
            });
            symbols.as_ref().clone()
        }
    }
}
