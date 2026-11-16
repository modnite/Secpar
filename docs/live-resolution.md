# Live resolution change

The in-stream Quality row can ask the host for a new stream size without ending
the session. That needs an engine that rebuilds its encoder at a new size inside
a running session, which is what `integrations/gamestream/patches/sunshine/0003-secpar-live-resolution.patch`
adds (`POST /api/secpar/resolution` on the engine's loopback interface).

## Why the patch is not in the 4.0.0 packages

The patched engine regressed engine start-up on the owner's Windows PC, so it
was left out of the packages until that is understood. What was seen, on
2026-11-15:

- One build of the patched engine died at start with an access violation.
- Another build reached encoder creation and aborted with
  `Creating encoder [av1_nvenc] ... NvEnc: encoding format is not supported by
  the gpu`.
- With either build the background part then reported
  `Bundled host stopped before it became ready`, and hosting never started.
- The unpatched engine from 13 November, in the same state folder and with the
  same configuration, starts, hosts and stays up.

So the engine shipped in the 4.0.0 packages is built without 0003. The window
and the stream component keep the Quality row; with this engine it reports that
the host cannot change the size, which is the honest state.

## Where the next attempt should start

From the evidence above, not from a fresh guess:

1. Reproduce on the same PC with the patched engine built from the current pin,
   and capture the access violation's faulting module if it recurs.
2. Look at how the patch touches encoder setup (`src/video.cpp`): the
   `av1_nvenc` message means the encode session is being opened with a format
   the GPU in use does not accept, which is what a rebuild-at-a-new-size path
   would do if it dropped or changed the format the first session used.
3. Compare the encoder configuration the patched build passes with the one the
   unpatched build passes for the same session, on that machine.
4. Only after the engine starts, hosts and survives a size change there does it
   go back into a package, with that run recorded here.
