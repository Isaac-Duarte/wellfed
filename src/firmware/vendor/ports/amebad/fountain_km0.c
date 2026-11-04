/* The PLWF106 boot test's KM0 main: boot KM4 and nothing else.
 *
 * Unlike the feeder's (firstboot_km0.c) it runs no boot trial, so it never
 * reads a confirmation record or clears bank 1's marker, and it writes and
 * erases no flash; the build's write audit holds it to that. It starts no
 * WLAN firmware and no flash-lease peer (KM4 takes no lease), and it leaves
 * the audio-shared pads unpowered, as the fountain uses none of them. A
 * bank-1 boot that fails stays failed until the bench recovers it: nothing
 * here gives way to bank 0.
 */
#include "ameba_soc.h"
#include "main.h"

int main(void)
{
    InterruptRegister((IRQ_FUN)IPC_INTHandler, IPC_IRQ_LP, (u32)IPCM4_DEV, 2);
    InterruptEn(IPC_IRQ_LP, 2);
    pmu_acquire_wakelock(PMU_OS);
    InterruptDis(UART_LOG_IRQ_LP);
    km4_flash_highspeed_init();
    km4_boot_on();
    vTaskStartScheduler();
    for (;;) {}
}
