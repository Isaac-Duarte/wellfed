# Speaker

The PLAF103 has a real speaker, not a piezo buzzer. Stock uses it to play a meal call the owner records in the app. Wellfed drives the same audio path with a few built-in tunes and the owner's own meal call, one at a time and always bounded. No feed decision reads the speaker, though its steps take time in the loop a feed shares ([device API](device-api.md)).

The built-in tunes and your own meal call have both been heard on our feeder. Which sounds play when is unsettled product policy, and a feed plays no tune. Status is on the [roadmap](../roadmap.md).

## What stock does

Recovered from the stock KM4 image (`DL_AUDIO`, `audio_play_http_file`):

- **Audio path.** Stock decodes AAC (a downloaded meal call, or one of up to 11 named prompts) and streams PCM through SPORT by GDMA into the codec, set up with `CODEC_Init(rate, 16-bit, mono flag, 8)` (application 8 is `APP_LINE_OUT`). The I2S PLL and divider follow the rate: 8, 16, 32, 44.1 or 48 kHz.
- **Amplifier.** PB23 (`0x37`), HIGH while it plays and LOW after: the amplifier enable.
- **Buzzer.** PA25, a separate active buzzer, pulsed HIGH by a one-shot 200-tick timer; a HIGH alone beeps ([evidence](#evidence)). The `feeder` firmware beeps it 200 ms on every button press, as stock does, when sound may play (`sound` on, volume above 0, inside the sound window). It has one pitch and loudness, so volume only mutes it at 0.
- **Clips in flash** (from a full-flash dump): a one-word count (3) at `0x400000`, and a 2 MiB FAT12 filesystem with 4 KiB clusters at `0x4B0000`. It holds `TEST1.AAC`–`TEST11.AAC` (no `TEST8`) and `COME_T~1.AAC`, probably the meal call, 17–78 KB each as ADTS AAC; the first is 44.1 kHz stereo. These clips stay on the device and never enter the repository.

## How Wellfed plays a tune

**The C shim** (`src/firmware/vendor/ports/amebad/example_entry.c`) opens the codec as stock does, at 16 kHz, 16-bit mono, line out, with SPORT and an 8-page GDMA ring of 16 ms pages. With no filled page ready, the DMA plays a zero page and counts an underrun, so the stream never stops by itself. Close stops the DMA, frees its channel, and stops SPORT and the codec output. Following the SDK's line-out path, `CODEC_Init` rewrites the audio LDO trim (the value KM0 already writes) and disables the PB28–PB31 pads, touching no pin this firmware uses.

**Tunes** (`wellfed-core/src/tone.rs`) are fixed note lists rendered as sines, with a 4 ms attack and 10 ms release so notes don't click, each under 6 s: `chime` (startup), `meal`, `coin`, `alert`, `test` (1 kHz for 1 s), `twinkle` and `key` (one 80 ms tick, BLE setup's press feedback). Loudness follows the `volume` setting on a decibel scale: 0 is silent, then 0.3 dB per step up to a peak of 16000 (about −6 dBFS) at 100. A linear scale made 40 and 80 sound almost alike.

**The meal call** (`wellfed-core/src/clip.rs`) is one owner clip, stored as 8 kHz 4-bit IMA ADPCM (two samples a byte), at most 32,256 bytes, about 8 s. The player decodes it a byte at a time from the mapped flash, with no buffer, and doubles each sample to 16 kHz, the second of each pair halfway to the next. Volume scales it on the tunes' decibel curve. A client converts the recording before upload ([meal call](schedules.md#meal-call)).

**The speaker task** (`wellfed-hal/src/audio.rs`, `wellfed-aud`, priority 2, normal role only):

- A one-slot mailbox holds the tune until playback ends, so a second request is `busy`, not queued.
- For each tune it opens the codec, sets PB23 HIGH, and plays 150 ms of silence while the amplifier settles. It streams the tune, waits at most 400 ms for the ring to drain, then sets PB23 LOW and closes. A ring that stops draining still ends at the tune's length plus 1 s, so the amplifier is never left on.
- The QA firmware and the bring-up `speaker` variant play the chime once the settings load, unless muted. The `feeder` firmware boots silently, and its loop steps the speaker itself.

Audio is independent of feeding: the task touches only the codec, its DMA and PB23, and only reads flash. The settings writer alone writes the clip's sectors, and the speaker and writer each take the sectors before use, so a call never reads a clip being rewritten, and an upload's next write waits for a call to end.

## Asking for a tune

`audio::request` queues a tune, and `audio::request_call(plays)` the meal call. They answer:

- `Started`: queued, not yet heard. The speaker opens the codec on its next step, after the device API has answered `Done`, so a codec that won't open reaches only the log (`error stage=open`).
- `Muted`: `sound` off, volume 0, or outside the sound window (every window is open while the clock is invalid).
- `Busy`: another tune plays, the settings are not loaded yet, or (for the call) no clip is stored.

The `feeder` loop asks for the call ahead of a plan's meal. The device API's `Play` request carries a tune, which the loop queues the same way, with more reasons for `Busy`: a feed queued or running, outputs claimed, a plan's meal being claimed, BLE setup, an install, a restart, or a tune that would still sound when a meal call's lead begins. A store write holds nothing back, since a tune writes no flash and reads no sector a writer changes. Setup mode silences whatever plays as it starts.

The speaker firmware builds with `wellfed-build build --device plaf103 --profile firstboot --firmware bringup --variant speaker`, or installs with `wellfed-bench ota NAME --build --firmware speaker`. It runs the tune task and plays the chime, with no motion; OTA, recovery and confirmation work as in every firmware.

## Records

`PLAUD1` lines: `start created=0|1`; `play tune=… volume=…`; `done tune=… ms=… underruns=… drained=0|1`; `muted tune=chime`; `error stage=open`, or `error stage=clip` when the call's clip was deleted or is being rewritten; and for the call, `play tune=call plays=… ms=… volume=…` and `done tune=call ms=… underruns=… drained=0|1`.

## Not done

Playing stock's own clips would need a read-only FAT reader, the SDK's Helix AAC decoder and a 44.1 kHz codec setup. The owner's meal call replaces them.

## Evidence

- **Meal call, on the speaker.** Observed (October 29, owner listening, hopper empty): plans calling a 2 s clip twice before a 1-portion meal, at volumes 10, 50 and 100. Each was clearly louder than the last, and each call finished with no underruns before its feed ran. An unattended run (October 28) kept the clip across a reboot, and with sound off logged `result=muted` and fed on time.
- **The buzzer.** Observed (October 28, owner listening, `feeder` `buzzer` variant v334): with the amplifier and codec off, a 200 ms HIGH on PA25 gave a clear beep, while 512 Hz and 2,048 Hz square waves sounded rough, as an active buzzer's own oscillator chopped by its supply would. The pad read back as driven. The normal build went back on afterwards.
- **Playback.** Observed (October 22, OTA v20 and v21, owner listening): the chime played on the first boot (`done tune=chime ms=624 underruns=0 drained=1`) and was heard clearly, as were `coin` and `twinkle`. A `coin` during `twinkle` got `busy`. On the decibel scale, volumes 40, 80 and 100 were clearly stepped and clean. With `sound` off a play request was refused `muted`. Settings were restored (sound on, volume 40, generation 2985). These requests went over the since-deleted MQTT transport to the same `audio::request`.
- **Amplifier settle.** Observed (October 24): a 60 ms settle swallowed a 50 ms tick whole, so the settle is 150 ms.
- **Linear volume.** Observed: on a linear scale, 40 and 80 sounded almost alike.
- **During feeds.** Observed (October 26): a tune played during feeds under load with `underruns=0`, and the feeds ended Done. Feeds play no tune since the owner's decision that day.
- **Host.** Modeled: `wellfed-core`'s `tone` tests (pitch, the volume scale, silence at 0, note edges, tune bounds) and `clip` tests (October 28: the ADPCM round trip, rate doubling, plays, volume 0, a failed read, codes at the rails without overflow); `wellfed-hal`'s `audio` tests against a fake codec ring (the chime waits for settings, mute, the amplifier on only while the codec is open, a failed open freeing the slot, a stalled ring still closing, a stored clip played with no underruns, the writer held off while it plays and holding the clip off in turn); `play` parsing.
