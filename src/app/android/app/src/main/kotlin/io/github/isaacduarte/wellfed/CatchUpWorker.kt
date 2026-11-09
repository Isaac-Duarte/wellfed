package io.github.isaacduarte.wellfed

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resume
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext

/**
 * The periodic catch-up: one round with every feeder in an engine of its
 * own. WorkManager runs it no more often than every 15 minutes, and far less
 * often in Doze, which the alert settings say.
 */
class CatchUpWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result =
        withContext(Dispatchers.Main) {
            val engine = BackgroundEngine(applicationContext)
            try {
                // The engine bounds the round itself.
                suspendCancellableCoroutine { waiting -> engine.round { waiting.resume(it) } }
            } finally {
                engine.destroy()
            }
            // A round that failed tries again at the next period, not sooner:
            // a retry's backoff would only bunch catch-ups up.
            Result.success()
        }

    companion object {
        private const val NAME = "wellfed-catch-up"
        private const val EVERY_MINUTES = 15L

        /**
         * Ask for the periodic catch-up, updating one already asked for in
         * place, so a new build's period or constraints take without
         * resetting when the next one runs. On any
         * network, not only Wi-Fi: a round must run to see the phone left
         * home. With a network constraint all the same, so Android holds the
         * job until this app may use the network, and a Doze window can't
         * turn a blocked socket into a failed catch-up at home.
         */
        fun schedule(context: Context) {
            val request =
                PeriodicWorkRequestBuilder<CatchUpWorker>(EVERY_MINUTES, TimeUnit.MINUTES)
                    .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                    .build()
            WorkManager.getInstance(context)
                .enqueueUniquePeriodicWork(NAME, ExistingPeriodicWorkPolicy.UPDATE, request)
        }
    }
}
