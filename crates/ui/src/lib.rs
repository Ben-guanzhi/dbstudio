#![recursion_limit = "256"]

pub mod components;
pub mod fold;
pub mod state;
pub mod url_scheme;
pub mod url_scheme_registry;
pub mod utils;

#[cfg(test)]
mod pagination_tests;

#[cfg(test)]
mod vim_tests;

#[cfg(test)]
mod help_tests;

#[cfg(test)]
mod fk_options_tests;

#[cfg(test)]
mod snapshot_eq_tests;

#[cfg(test)]
mod fold_tests;
