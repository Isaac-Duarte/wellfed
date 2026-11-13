# Evidence levels

Every finding in these docs carries one of these labels, saying how the fact was established and on which unit, never that repeating the experiment is safe or authorized.

| Label | Also written as | Meaning |
| --- | --- | --- |
| **Observed** | physical, physical observation | Seen on a real unit: part markings, UART output, a meter reading, or a recorded physical operation. Holds for that unit and that image unless stated otherwise. |
| **Recovered** | static analysis, static | Traced in code or a design without running it: stock firmware instructions, the vendor app, the SDK, or Wellfed's own code. No electrical verification of the wiring or timing. |
| **Modeled** | simulation, simulated, host-tested | Host tests, the simulator, or execution against supplied register/ROM/NOR models. Says nothing about hardware reliability. |
| **External** | peer project | Reported by another project, usually on a different model (see [references](references.md)). A lead for our own checks, never a fact about our devices until we observe it. |
| **Unknown** | — | Not established. Unknown flash ranges are not free space, and unknown pins are not unused. |

Most findings come from one research unit, and apply to other units of the same model only when they say so. A finding on one model never carries over to another by default, even one with the same SoC.
