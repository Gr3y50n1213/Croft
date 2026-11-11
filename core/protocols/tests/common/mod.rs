// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Helpers shared by the adapter tests: the behaviour test that runs against every adapter, and a
//! tiny raw JMAP client for test servers (used to seed mail and to act as "another client").
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

pub mod behaviour;
pub mod rawjmap;
