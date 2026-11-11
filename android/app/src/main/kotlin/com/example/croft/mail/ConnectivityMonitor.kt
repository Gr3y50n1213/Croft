// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.mail

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.distinctUntilChanged

/**
 * Whether the device has a network, as a flow: the platform's own connectivity callback in the app ([AndroidConnectivity]), a
 * hand-driven flow in tests. [MailSession.followConnectivity] turns each change into `set_network_available` on the core, which
 * shows the Offline banner and holds every change until the network is back.
 */
fun interface Connectivity {
    /** Emits the current state first, then every change. Never completes. */
    fun available(): Flow<Boolean>
}

/**
 * The platform callback (`ConnectivityManager.registerDefaultNetworkCallback`, which needs ACCESS_NETWORK_STATE). The network
 * counts as there when the default network says it can reach the internet; a captive portal still says so, and the server's own
 * answer then decides (the core backs off on its own).
 */
class AndroidConnectivity(context: Context) : Connectivity {
    private val manager = context.applicationContext.getSystemService(ConnectivityManager::class.java)

    override fun available(): Flow<Boolean> = callbackFlow {
        val manager = manager
        if (manager == null) {
            // No connectivity service (a stripped test environment): assume there is a network and let the sockets say otherwise.
            trySend(true)
            awaitClose { }
            return@callbackFlow
        }
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) { trySend(hasInternet(manager, network)) }

            override fun onLost(network: Network) { trySend(manager.activeNetwork?.let { hasInternet(manager, it) } ?: false) }

            override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
                trySend(capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET))
            }
        }
        trySend(manager.activeNetwork?.let { hasInternet(manager, it) } ?: false)
        manager.registerDefaultNetworkCallback(callback)
        awaitClose { manager.unregisterNetworkCallback(callback) }
    }.distinctUntilChanged()

    private fun hasInternet(manager: ConnectivityManager, network: Network): Boolean =
        manager.getNetworkCapabilities(network)?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true
}

/** Collects [connectivity] and tells the repository about every change. Runs until cancelled (a `LaunchedEffect` of the app). */
suspend fun MailSession.followConnectivity(connectivity: Connectivity) {
    connectivity.available().collect { available ->
        try {
            repository.setNetworkAvailable(available)
        } catch (e: MailException) {
            // The core refused (it is closing): the next change tells it again.
        }
    }
}
