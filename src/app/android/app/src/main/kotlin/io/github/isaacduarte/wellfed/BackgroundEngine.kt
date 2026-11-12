package io.github.isaacduarte.wellfed

import android.content.Context
import android.os.Handler
import android.os.Looper
import com.dexterous.flutterlocalnotifications.FlutterLocalNotificationsPlugin
import com.haberey.flutter.nsd_android.NsdAndroidPlugin
import com.it_nomads.fluttersecurestorage.FlutterSecureStoragePlugin
import io.flutter.FlutterInjector
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.embedding.engine.dart.DartExecutor
import io.flutter.plugin.common.MethodChannel

/**
 * A Flutter engine for catch-ups with no screen: the periodic job's and the
 * instant service's. Built with no automatic registration, as the plugins'
 * own background engines are not: those attach every plugin the app has,
 * and the Bluetooth plugin keeps its controller and its open links in
 * statics, so a second engine attaching it takes the app's scan and links
 * over, and its detach disconnects them. A catch-up starting during a
 * Bluetooth setup would cut it. This engine gets only what a catch-up
 * needs: notifications, the client secret and NSD.
 *
 * Runs on the main thread, as every engine must.
 */
class BackgroundEngine(private val context: Context) {
    // Before the engine starts Dart, as MainActivity does: WorkManager and
    // the boot receiver start this in a process no activity ever ran in,
    // and a catch-up through an https server panics on a verifier never
    // given the context. Starting it again in a process that has changes
    // nothing.
    init {
        TlsVerifier.start(context.applicationContext)
    }

    private val engine = FlutterEngine(context, null, false)
    private val channel = MethodChannel(engine.dartExecutor.binaryMessenger, CHANNEL)
    private val main = Handler(Looper.getMainLooper())

    // Rounds asked for before Dart could hear them.
    private var ready = false
    private val waiting = mutableListOf<(Boolean) -> Unit>()

    init {
        engine.plugins.add(FlutterLocalNotificationsPlugin())
        engine.plugins.add(FlutterSecureStoragePlugin())
        engine.plugins.add(NsdAndroidPlugin())
        channel.setMethodCallHandler { call, result ->
            when (call.method) {
                "ready" -> {
                    // The answer first: Dart takes the directory from it
                    // before any round may start.
                    result.success(context.filesDir.path)
                    ready = true
                    val asked = waiting.toList()
                    waiting.clear()
                    main.post { asked.forEach { send(it) } }
                }
                "network" -> result.success(WifiNetworks.use(context, this))
                else -> result.notImplemented()
            }
        }

        val loader = FlutterInjector.instance().flutterLoader()
        loader.startInitialization(context)
        loader.ensureInitializationComplete(context, null)
        engine.dartExecutor.executeDartEntrypoint(
            DartExecutor.DartEntrypoint(loader.findAppBundlePath(), ENTRY_LIBRARY, ENTRY),
        )
    }

    /**
     * One round of catch-ups with every feeder; `done` hears once whether it
     * ran, and hears no after [ROUND_LIMIT_MS] whatever Dart is doing, so a
     * round stuck on a socket or a discovery never holds up the next.
     */
    fun round(finished: (Boolean) -> Unit) {
        var heard = false
        val done = { ran: Boolean ->
            if (!heard) {
                heard = true
                finished(ran)
            }
        }
        main.postDelayed({ done(false) }, ROUND_LIMIT_MS)
        if (ready) send(done) else waiting.add(done)
    }

    private fun send(done: (Boolean) -> Unit) {
        channel.invokeMethod(
            "round",
            null,
            object : MethodChannel.Result {
                override fun success(result: Any?) {
                    ended(done, true)
                }

                override fun error(code: String, message: String?, details: Any?) {
                    ended(done, false)
                }

                override fun notImplemented() {
                    ended(done, false)
                }
            },
        )
    }

    // Each round binds the network afresh: the phone may have moved since.
    private fun ended(done: (Boolean) -> Unit, ran: Boolean) {
        WifiNetworks.release(context, this)
        main.post { done(ran) }
    }

    fun destroy() {
        waiting.forEach { it(false) }
        waiting.clear()
        WifiNetworks.release(context, this)
        engine.destroy()
    }

    private companion object {
        const val CHANNEL = "io.github.isaacduarte.wellfed/background"
        const val ENTRY_LIBRARY = "package:wellfed/platform/background.dart"
        const val ENTRY = "background"

        // Each catch-up is bounded at 20 s on the feeder, plus NSD's 6 s look
        // when the first try misses: room for a few feeders.
        const val ROUND_LIMIT_MS = 4 * 60 * 1000L
    }
}
