package io.github.isaacduarte.wellfed

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkInfo
import java.net.Inet4Address

/**
 * The Wi-Fi network as a catch-up needs it: which network the phone is on, by
 * its addresses, so Rust can tell the feeder's home network from another
 * with no location permission (a network's name needs one), and the process
 * bound to it, so Rust's sockets reach the feeder.
 *
 * Read by its transport rather than as the default network: while the Wi-Fi
 * has no internet Android keeps mobile data as the default, and reading that
 * would call a phone at home away.
 */
object WifiNetworks {
    // The network a background engine bound the process to, while any uses
    // it. What this did not bind it never unbinds: MainActivity holds the
    // process on the feeder's setup network even once that network is lost,
    // so that a frame with the Wi-Fi password fails rather than goes out on
    // the home LAN.
    private var ours: Network? = null
    private val users = mutableSetOf<Any>()

    // The network MainActivity bound the process to, which wins over ours: a
    // background round that ends never unbinds what the activity bound,
    // even the same network it bound itself first.
    private var activity: Network? = null

    /**
     * Binds the process to the Wi-Fi network if nothing is bound, and
     * describes the network the process uses. Null with no Wi-Fi, or while
     * Android keeps this app off the network for now (Doze, App Standby),
     * when a failed session says nothing about the feeder. `user` keeps the
     * binding until it calls [release].
     */
    @Synchronized
    fun use(context: Context, user: Any): Map<String, Any?>? {
        val connectivity = context.getSystemService(ConnectivityManager::class.java) ?: return null
        users.add(user)
        val bound = connectivity.boundNetworkForProcess
        val network = bound ?: wifi(connectivity) ?: return null
        if (blocked(connectivity, network)) return null
        val found = describe(connectivity.getLinkProperties(network)) ?: return null
        if (bound == null && connectivity.bindProcessToNetwork(network)) ours = network
        return found
    }

    /** `user` is done with the network. Unbinds only what no one else claims. */
    @Synchronized
    fun release(context: Context, user: Any) {
        if (!users.remove(user) || users.isNotEmpty()) return
        val connectivity = context.getSystemService(ConnectivityManager::class.java) ?: return
        if (activity == null && ours != null && connectivity.boundNetworkForProcess == ours) {
            connectivity.bindProcessToNetwork(null)
        }
        ours = null
    }

    /**
     * MainActivity binds the process to `network`, or lets go with null. Its
     * binding is its own from then on, whoever bound first. When it lets go
     * while a background round still uses the Wi-Fi, the round keeps it.
     * Whether Android bound it.
     */
    @Synchronized
    fun bindForActivity(connectivity: ConnectivityManager, network: Network?): Boolean {
        activity = network
        if (network != null) {
            ours = null
            return connectivity.bindProcessToNetwork(network)
        }
        val kept = if (users.isEmpty()) null else wifi(connectivity)
        ours = kept
        return connectivity.bindProcessToNetwork(kept)
    }

    /** The network the app's own engine uses now, described, binding nothing. */
    fun current(connectivity: ConnectivityManager): Map<String, Any?>? {
        val network = connectivity.boundNetworkForProcess ?: wifi(connectivity) ?: return null
        if (blocked(connectivity, network)) return null
        return describe(connectivity.getLinkProperties(network))
    }

    @Suppress("DEPRECATION") // The one way to list a network Android does not default to.
    private fun wifi(connectivity: ConnectivityManager): Network? =
        connectivity.allNetworks.firstOrNull { network ->
            connectivity.getNetworkCapabilities(network)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
        }

    @Suppress("DEPRECATION") // Nothing newer says this for one app on demand.
    private fun blocked(connectivity: ConnectivityManager, network: Network): Boolean =
        connectivity.getNetworkInfo(network)?.detailedState == NetworkInfo.DetailedState.BLOCKED

    private fun describe(properties: LinkProperties?): Map<String, Any?>? {
        properties ?: return null
        val gateway =
            properties.routes
                .firstOrNull { it.isDefaultRoute && it.gateway is Inet4Address }
                ?.gateway
                ?.hostAddress
                ?: return null
        val link = properties.linkAddresses.firstOrNull { it.address is Inet4Address } ?: return null
        return mapOf(
            "gateway" to gateway,
            "subnet" to subnetOf(link.address.address, link.prefixLength),
            "dns" to properties.dnsServers.mapNotNull { it.hostAddress },
            "domains" to properties.domains,
        )
    }

    private fun subnetOf(bytes: ByteArray, prefix: Int): String {
        val bits = bytes.fold(0) { acc, byte -> (acc shl 8) or (byte.toInt() and 0xff) }
        val masked = if (prefix == 0) 0 else bits and (-1 shl (32 - prefix))
        val octets = (3 downTo 0).map { (masked ushr (it * 8)) and 0xff }
        return octets.joinToString(".") + "/" + prefix
    }
}
