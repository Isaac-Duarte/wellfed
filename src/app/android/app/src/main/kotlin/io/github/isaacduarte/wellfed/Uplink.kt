package io.github.isaacduarte.wellfed

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest

/**
 * The network Rust's relay reaches the owner's server by while the Wi-Fi the
 * process is bound to has no way out.
 *
 * The process stays bound to the Wi-Fi so the feeders' sessions reach them,
 * and Android keeps mobile data as the default network while that Wi-Fi has
 * no validated internet (a hotspot with no uplink, a captive portal). Then,
 * and only then, Rust hears the default network's handle, and sends a
 * request to a server off the home network through its relay on that
 * network; it hears zero otherwise. Feeder sessions never use it.
 */
object Uplink {
    init {
        System.loadLibrary("wellfed_app_bridge")
    }

    @JvmStatic private external fun set(handle: Long)

    private var connectivity: ConnectivityManager? = null

    // The network Android routes this app's traffic over by default.
    private var default: Network? = null

    /** Start watching, once per process; a second call changes nothing. */
    @Synchronized
    fun start(context: Context) {
        if (connectivity != null) return
        val manager = context.getSystemService(ConnectivityManager::class.java) ?: return
        connectivity = manager
        manager.registerDefaultNetworkCallback(
            object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) {
                    synchronized(this@Uplink) { default = network }
                    refresh()
                }

                override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) = refresh()

                override fun onLost(network: Network) {
                    synchronized(this@Uplink) { if (default == network) default = null }
                    refresh()
                }
            },
        )
        // The bound Wi-Fi gains and loses its way out on its own, as a
        // hotspot's uplink comes and goes. Listening keeps no network up.
        val wifi =
            NetworkRequest.Builder()
                .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
                .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
                .build()
        manager.registerNetworkCallback(
            wifi,
            object : ConnectivityManager.NetworkCallback() {
                override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) = refresh()

                override fun onLost(network: Network) = refresh()
            },
        )
    }

    /** Tell Rust again which network to go out on: after the process binds elsewhere. */
    @Synchronized
    fun refresh() {
        val manager = connectivity ?: return
        val bound = manager.boundNetworkForProcess
        val uplink = default
        val handle =
            if (
                bound != null &&
                uplink != null &&
                uplink != bound &&
                !validated(manager, bound) &&
                validated(manager, uplink)
            ) {
                uplink.networkHandle
            } else {
                0L
            }
        set(handle)
    }

    private fun validated(manager: ConnectivityManager, network: Network): Boolean {
        val capabilities = manager.getNetworkCapabilities(network) ?: return false
        return capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
    }
}
