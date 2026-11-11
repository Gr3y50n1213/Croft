// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp

/**
 * The first screen: proves the Rust core is loaded and answering. `info` is null when the library
 * could not be loaded or speaks another FFI API version.
 */
@Composable
fun HelloScreen(info: RustInfo?) {
    Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Column(
            modifier = Modifier.safeDrawingPadding().padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterVertically),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(text = stringResource(R.string.app_name), style = MaterialTheme.typography.displayMedium, color = MaterialTheme.colorScheme.primary)
            if (info != null) {
                Text(text = stringResource(R.string.core_version, info.croftVersion), style = MaterialTheme.typography.titleMedium)
                Text(text = stringResource(R.string.ffi_api_version, info.ffiApiVersion.toString()), style = MaterialTheme.typography.bodyMedium)
            } else {
                Text(text = stringResource(R.string.core_unavailable), color = MaterialTheme.colorScheme.error)
            }
        }
    }
}
