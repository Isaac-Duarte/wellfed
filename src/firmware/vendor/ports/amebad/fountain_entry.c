/* The PLWF106 boot test's hooks for the vendor KM4 main: the Rust staticlib
 * (src/firmware/wellfed/fountain) is the whole application. There is no
 * network, Bluetooth, audio, socket or store code here: the image starts
 * nothing but the Rust entry, which latches the pump off first.
 */
#include "ameba_soc.h"
#include "FreeRTOS.h"
#include "task.h"
#include <platform/platform_stdlib.h>

void wellfed_app_start(void);

/* Vendor prints stay outside the log ring, which the builder's KM4 link
 * wraps for each: a print from another object first lets a logged line
 * partly on the wire finish (wellfed_log_settle), so the two never mix.
 * Naked, because the print is variadic and reads its arguments from where
 * the caller left them; the flag is read in RAM, as the feeder's entry does
 * (example_entry.c). */
#define WELLFED_PRINT_WRAP(name)                                  \
    IMAGE2_RAM_TEXT_SECTION                                       \
    __attribute__((naked)) void __wrap_##name(void)               \
    {                                                             \
        __asm volatile(                                           \
            "movw ip, #:lower16:wellfed_log_mid_line\n\t"         \
            "movt ip, #:upper16:wellfed_log_mid_line\n\t"         \
            "ldrb ip, [ip]\n\t"                                    \
            "cmp ip, #0\n\t"                                       \
            "beq 1f\n\t"                                           \
            "push {r0, r1, r2, r3, ip, lr}\n\t"                    \
            "bl wellfed_log_settle\n\t"                            \
            "pop {r0, r1, r2, r3, ip, lr}\n"                       \
            "1:\n\t"                                               \
            "b.w __real_" #name "\n\t");                           \
    }

WELLFED_PRINT_WRAP(DiagPrintf)
WELLFED_PRINT_WRAP(_rtl_printf)

/* The builder sets configUSE_TICK_HOOK for every Rust image. The boot test
 * keeps no hang crumb: it arms no watchdog, so no reset follows a stall
 * for a crumb to explain. */
void vApplicationTickHook(void)
{
}

void pre_example_entry(void)
{
}

void example_entry(void)
{
    wellfed_app_start();
}
