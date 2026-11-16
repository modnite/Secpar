package win.secpar.app;

import android.content.Context;
import android.content.SharedPreferences;
import android.preference.PreferenceManager;
import android.view.Display;
import android.view.WindowManager;

import com.limelight.binding.video.MediaCodecHelper;
import com.limelight.preferences.GlPreferences;
import com.limelight.preferences.PreferenceConfiguration;

import java.util.ArrayList;
import java.util.List;

/**
 * The stream choices this device can actually use, and the preferences the
 * stream reads them from. Every list is worked out with Moonlight's own
 * capability code, so a value is never offered that this device cannot decode:
 * the resolution ladder comes from {@code StreamSettings}' display and decoder
 * checks, the frame rates from the display's modes, and the video formats from
 * the decoders Moonlight would pick. Nothing here draws; SecparActivity shows it.
 */
final class StreamChoices {
    /*
     * PreferenceConfiguration's own keys, with the same value formats. They are
     * package-private there, so they are repeated here; the stream reads the
     * values through that class, not through this one.
     */
    static final String RESOLUTION_KEY = "list_resolution";
    static final String FPS_KEY = "list_fps";
    /** An int in kbps, as Moonlight's slider stored it. */
    static final String BITRATE_KEY = "seekbar_bitrate_kbps";
    static final String VIDEO_FORMAT_KEY = "video_format";
    static final String FRAME_PACING_KEY = "frame_pacing";
    static final String AUDIO_KEY = "list_audio_config";
    static final String STRETCH_KEY = "checkbox_stretch_video";
    static final String UNLOCK_FPS_KEY = "checkbox_unlock_fps";

    /** PreferenceConfiguration's defaults, repeated because they are package-private. */
    private static final String DEFAULT_RESOLUTION = "1280x720";
    private static final String DEFAULT_FPS = "60";
    private static final String DEFAULT_VIDEO_FORMAT = "auto";
    private static final String DEFAULT_FRAME_PACING = "latency";
    private static final String DEFAULT_AUDIO = "2";

    /** Bitrates offered in kbps, inside Moonlight's own 500 to 150000 range. */
    private static final int[] BITRATE_LADDER = {5000, 10000, 15000, 20000, 30000, 50000, 75000, 100000, 150000};

    /** One value a row can be set to, as it is stored and as it is shown. */
    static final class Option {
        final String value;
        final String label;

        Option(String value, String label) {
            this.value = value;
            this.label = label;
        }
    }

    final List<Option> resolutions = new ArrayList<>();
    final List<Option> frameRates = new ArrayList<>();
    final List<Option> videoFormats = new ArrayList<>();
    final List<Option> bitrates = new ArrayList<>();

    private StreamChoices() {
    }

    /** Works the device's choices out; call it off the main thread. */
    static StreamChoices read(Context context) {
        StreamChoices choices = new StreamChoices();
        addResolutions(context, choices);
        addFrameRates(context, choices);
        addVideoFormats(context, choices);
        addBitrates(context, choices);
        return choices;
    }

    // ---- What this device can play

    private static void addResolutions(Context context, StreamChoices choices) {
        int maxWidth = maxDecoderWidth(context);
        // 720p is never dropped, the same as Moonlight's own screen.
        addResolution(choices, "640x360", 640, maxWidth);
        addResolution(choices, "854x480", 854, maxWidth);
        addResolution(choices, "1280x720", 1280, maxWidth);
        addResolution(choices, "1920x1080", 1920, maxWidth);
        addResolution(choices, "2560x1440", 2560, maxWidth);
        addResolution(choices, "3840x2160", 3840, maxWidth);
    }

    private static void addResolution(StreamChoices choices, String value, int width, int maxWidth) {
        if (maxWidth == 0 || width <= maxWidth || width <= 1280) {
            choices.resolutions.add(new Option(value, value.replace("x", " x ")));
        }
    }

    /**
     * The widest picture Moonlight would hand to a decoder: the display's own
     * modes and the width range of the first AVC and HEVC decoders, exactly as
     * StreamSettings decides which resolution entries to keep. Zero means the
     * device told us nothing, and the whole ladder stays offered.
     */
    private static int maxDecoderWidth(Context context) {
        Display display = display(context);
        if (display == null) {
            return 0;
        }
        int max = 0;
        try {
            for (Display.Mode mode : display.getSupportedModes()) {
                int width = Math.max(mode.getPhysicalWidth(), mode.getPhysicalHeight());
                int height = Math.min(mode.getPhysicalWidth(), mode.getPhysicalHeight());
                if ((width >= 3840 || height >= 2160) && max < 3840) {
                    max = 3840;
                } else if ((width >= 2560 || height >= 1440) && max < 2560) {
                    max = 2560;
                } else if ((width >= 1920 || height >= 1080) && max < 1920) {
                    max = 1920;
                }
            }
        } catch (RuntimeException e) {
            // A display that cannot list its modes tells us nothing extra.
        }
        int avc = decoderWidth(context, "video/avc");
        if (avc >= 1280 && max < avc) {
            max = avc;
        }
        int hevc = decoderWidth(context, "video/hevc");
        if (hevc >= 1280 && max < hevc) {
            max = hevc;
        }
        return max;
    }

    /** The 3840, 2560 or 1920 a decoder reports it can take, or 0. */
    private static int decoderWidth(Context context, String mime) {
        android.media.MediaCodecInfo decoder = decoder(context, mime);
        if (decoder == null) {
            return 0;
        }
        try {
            android.util.Range<Integer> widths = decoder.getCapabilitiesForType(mime).getVideoCapabilities().getSupportedWidths();
            if (!widths.contains(1280)) {
                // Nonsense or nothing usable from this decoder, as Moonlight treats it.
                return 0;
            }
            if (widths.contains(3840)) {
                return 3840;
            }
            if (widths.contains(1920)) {
                return 1920;
            }
            return 1280;
        } catch (RuntimeException e) {
            return 0;
        }
    }

    private static void addFrameRates(Context context, StreamChoices choices) {
        choices.frameRates.add(new Option("30", "30 fps"));
        choices.frameRates.add(new Option("60", "60 fps"));
        // Moonlight gives a little room in case the refresh rate is rounded down.
        boolean unlock = prefs(context).getBoolean(UNLOCK_FPS_KEY, false);
        float refresh = maxRefresh(context);
        if (unlock || refresh >= 88) {
            choices.frameRates.add(new Option("90", "90 fps"));
        }
        if (unlock || refresh >= 118) {
            choices.frameRates.add(new Option("120", "120 fps"));
        }
    }

    private static float maxRefresh(Context context) {
        Display display = display(context);
        if (display == null) {
            return 60;
        }
        float max = display.getRefreshRate();
        try {
            for (Display.Mode mode : display.getSupportedModes()) {
                max = Math.max(max, mode.getRefreshRate());
            }
        } catch (RuntimeException e) {
            // Keep the mode the display is in now.
        }
        return max;
    }

    private static void addVideoFormats(Context context, StreamChoices choices) {
        choices.videoFormats.add(new Option("auto", "Automatic"));
        if (decoder(context, "video/av01") != null) {
            choices.videoFormats.add(new Option("forceav1", "AV1"));
        }
        if (decoder(context, "video/hevc") != null) {
            choices.videoFormats.add(new Option("forceh265", "HEVC"));
        }
        choices.videoFormats.add(new Option("neverh265", "H.264"));
    }

    /** The decoder Moonlight would use for this format, or null if it has none. */
    private static android.media.MediaCodecInfo decoder(Context context, String mime) {
        try {
            MediaCodecHelper.initialize(context, GlPreferences.readPreferences(context).glRenderer);
            return MediaCodecHelper.findFirstDecoder(mime);
        } catch (RuntimeException | LinkageError e) {
            return null;
        }
    }

    private static void addBitrates(Context context, StreamChoices choices) {
        int current = bitrate(context);
        for (int kbps : BITRATE_LADDER) {
            choices.bitrates.add(option(kbps));
        }
        // A rate saved by Moonlight's own slider may sit between the ladder's
        // steps; it is shown as it is rather than rounded to a neighbour.
        for (Option option : choices.bitrates) {
            if (option.value.equals(String.valueOf(current))) {
                return;
            }
        }
        for (int i = 0; i < choices.bitrates.size(); i++) {
            if (Integer.parseInt(choices.bitrates.get(i).value) > current) {
                choices.bitrates.add(i, option(current));
                return;
            }
        }
        choices.bitrates.add(option(current));
    }

    private static Option option(int kbps) {
        String label = kbps % 1000 == 0 ? (kbps / 1000) + " Mbps" : kbps + " kbps";
        return new Option(String.valueOf(kbps), label);
    }

    // ---- The preferences the stream reads

    static SharedPreferences prefs(Context context) {
        return PreferenceManager.getDefaultSharedPreferences(context);
    }

    static String resolution(Context context) {
        return prefs(context).getString(RESOLUTION_KEY, DEFAULT_RESOLUTION);
    }

    static String frameRate(Context context) {
        return prefs(context).getString(FPS_KEY, DEFAULT_FPS);
    }

    static String videoFormat(Context context) {
        return prefs(context).getString(VIDEO_FORMAT_KEY, DEFAULT_VIDEO_FORMAT);
    }

    static String framePacing(Context context) {
        return prefs(context).getString(FRAME_PACING_KEY, DEFAULT_FRAME_PACING);
    }

    static String audio(Context context) {
        return prefs(context).getString(AUDIO_KEY, DEFAULT_AUDIO);
    }

    static boolean stretch(Context context) {
        return prefs(context).getBoolean(STRETCH_KEY, false);
    }

    /** The rate the stream will use, the saved one or the default for the picture. */
    static int bitrate(Context context) {
        SharedPreferences prefs = prefs(context);
        // Moonlight's old key stored Mbps; its default is worked out from the
        // chosen resolution and frame rate.
        int kbps = prefs.getInt(BITRATE_KEY, prefs.getInt("seekbar_bitrate", 0) * 1000);
        return kbps == 0 ? PreferenceConfiguration.getDefaultBitrate(context) : kbps;
    }

    static void put(Context context, String key, String value) {
        prefs(context).edit().putString(key, value).apply();
    }

    static void putBitrate(Context context, int kbps) {
        prefs(context).edit().putInt(BITRATE_KEY, kbps).apply();
    }

    static void putStretch(Context context, boolean stretched) {
        prefs(context).edit().putBoolean(STRETCH_KEY, stretched).apply();
    }

    /** The label for a stored value, or the value itself when it is not in the list. */
    static String label(List<Option> options, String value, String otherwise) {
        for (Option option : options) {
            if (option.value.equals(value)) {
                return option.label;
            }
        }
        return otherwise;
    }

    private static Display display(Context context) {
        WindowManager windows = (WindowManager) context.getSystemService(Context.WINDOW_SERVICE);
        return windows == null ? null : windows.getDefaultDisplay();
    }
}
