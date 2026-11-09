package io.github.isaacduarte.wellfed

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.util.Log

/**
 * The one foreground service, for two reasons that may overlap:
 * - a run (an install, an update, a restore) that lives in the app's own
 *   engine and must outlive the screen. It ends with the task, as a run
 *   cannot go on without the engine it lives in;
 * - "instant while home", which catches up with every feeder each minute in
 *   an engine of its own ([BackgroundEngine]) and outlives the task.
 *
 * It holds a partial wake lock and a Wi-Fi lock while it runs: a locked
 * phone otherwise dozes, and the run's sockets or the minute's catch-up
 * stall until someone wakes it. Its own code, not a plugin's: the plugins'
 * services start a Flutter engine with every plugin attached, which takes the
 * Bluetooth plugin's links over from the app.
 */
class KeepAliveService : Service() {
    private val main = Handler(Looper.getMainLooper())
    private var wake: PowerManager.WakeLock? = null
    private var wifi: WifiManager.WifiLock? = null
    private var engine: BackgroundEngine? = null
    private var rounding = false

    private val tick =
        object : Runnable {
            override fun run() {
                round()
                main.postDelayed(this, INSTANT_EVERY_MS)
            }
        }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Promptly, every time, even to stop: Android ends an app that asked
        // for a foreground service and did not start one.
        startForeground(NOTIFICATION_ID, notification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        pending = maxOf(0, pending - 1)
        live = this
        apply()
        // Not restarted by Android after a kill: the app brings instant mode
        // back when it opens, and a run never survives its engine.
        return START_NOT_STICKY
    }

    // A run's Dart steps live in the app's engine, which goes with the task
    // and never says they ended: the engine's holds on catch-ups go too, or
    // they would stay off for the two hours a hold takes to lapse. A run's
    // thread still alive in Rust keeps its own.
    override fun onTaskRemoved(rootIntent: Intent?) {
        run = null
        RustHolds.clear()
        apply()
    }

    override fun onDestroy() {
        if (live === this) live = null
        main.removeCallbacks(settle)
        letGo()
        super.onDestroy()
    }

    // Re-run on every start and every change, so the locks and the engine
    // never outlast a reason to hold them.
    private fun apply() {
        main.removeCallbacks(settle)
        if (run == null && !instant) {
            letGo()
            // A start still on its way runs onStartCommand, which stops it,
            // and one that never arrives is given up on after a while.
            if (pending == 0) stopSelf() else main.postDelayed(settle, PENDING_LIMIT_MS)
            return
        }

        if (wake == null) {
            wake =
                getSystemService(PowerManager::class.java)
                    ?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "wellfed:keep-alive")
                    ?.apply { acquire() }
        }
        if (wifi == null) {
            @Suppress("DEPRECATION") // The mode that keeps the radio up with the screen off.
            wifi =
                getSystemService(WifiManager::class.java)
                    ?.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "wellfed:keep-alive")
                    ?.apply { acquire() }
        }
        getSystemService(NotificationManager::class.java)?.notify(NOTIFICATION_ID, notification())

        if (instant && engine == null) {
            engine = BackgroundEngine(applicationContext)
            main.post(tick)
        } else if (!instant && engine != null) {
            main.removeCallbacks(tick)
            engine?.destroy()
            engine = null
        }
    }

    // The locks and the instant engine go as soon as nothing holds, even
    // while the service waits for a start still on its way.
    private fun letGo() {
        main.removeCallbacks(tick)
        engine?.destroy()
        engine = null
        wake?.takeIf { it.isHeld }?.release()
        wake = null
        wifi?.takeIf { it.isHeld }?.release()
        wifi = null
    }

    // A start counted in `pending` that never reached onStartCommand: given
    // up on, so the service still stops.
    private val settle =
        Runnable {
            pending = 0
            apply()
        }

    // One round at a time: a slow one (a feeder gone, NSD looking) makes the
    // next minute's skip rather than pile up.
    private fun round() {
        val engine = engine ?: return
        if (rounding) return
        rounding = true
        engine.round { rounding = false }
    }

    private fun notification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        manager?.createNotificationChannel(
            NotificationChannel(CHANNEL, "Running in the background", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shown while an install runs, or while alerts are instant at home."
            },
        )
        val open =
            packageManager.getLaunchIntentForPackage(packageName)?.let {
                PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE)
            }
        val (title, text) =
            when (val running = run) {
                null -> "Watching your feeders" to "Alerts arrive within a minute while this phone is home."
                else -> running to "Keep Wellfed open until it finishes."
            }
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(applicationInfo.icon)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .build()
    }

    companion object {
        private const val TAG = "wellfed"
        private const val CHANNEL = "keep-alive"

        // Below every alert's id, which Rust keeps at 2^30 and above.
        private const val NOTIFICATION_ID = 7

        // A feeder's alerts are decided on its own fixed ticks; a minute
        // here keeps an urgent one within about a minute of the feeder.
        private const val INSTANT_EVERY_MS = 60_000L

        // Android must deliver a start within seconds or it ends the app, so
        // a start not delivered by then never will be.
        private const val PENDING_LIMIT_MS = 15_000L

        // What the service is for, and its state. Changed on the main thread
        // only.
        private var run: String? = null
        private var instant = false

        // Starts asked for that onStartCommand has not run yet, and the
        // service once one has.
        private var pending = 0
        private var live: KeepAliveService? = null

        /**
         * Whether instant mode runs now. It may be on in the settings and not
         * run: Android stopped the service, the phone restarted, or the app's
         * process is new. Opening the app starts it again.
         */
        fun instantRunning(): Boolean = instant && live?.engine != null

        /**
         * Hold the process for a run, saying `text`, or let go with `null`.
         * Whether Android started the service.
         */
        fun hold(context: Context, text: String?): Boolean {
            run = text
            return update(context)
        }

        /** Turn "instant while home" on or off. Whether Android went along. */
        fun setInstant(context: Context, on: Boolean): Boolean {
            instant = on
            val went = update(context)
            if (!went) instant = false
            return went
        }

        private fun update(context: Context): Boolean {
            if (run == null && !instant) {
                // Never stopService: a start still on its way would end
                // before its startForeground, and Android ends the app for
                // that. The service lets go of its locks at once and stops
                // itself, from onStartCommand if a start is on its way.
                live?.apply()
                return true
            }
            return try {
                context.startForegroundService(Intent(context, KeepAliveService::class.java))
                pending += 1
                true
            } catch (error: IllegalStateException) {
                // Android refuses a foreground service to an app it does
                // not see in front, and says so this way.
                Log.w(TAG, "the keep-alive service was refused: $error")
                false
            }
        }
    }
}
