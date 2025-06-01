// SPDX-FileCopyrightText: 2025 Fredrik Lanker <fredrik@lanker.se>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::AppState;
use axum::Router;
use std::sync::Arc;

pub mod handlers;

pub fn configure(state: &Arc<AppState>) -> Router {
    Router::new().nest("/api/", handlers::configure(state))
}
