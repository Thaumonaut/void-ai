package com.kaira.voicelab;

import android.content.Context;
import android.content.Intent;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.speech.RecognitionListener;
import android.speech.RecognizerIntent;
import android.speech.SpeechRecognizer;
import java.util.ArrayList;

/** Drives Android's built-in SpeechRecognizer on the main thread and exposes the
 *  result via static fields that the Rust side polls over JNI (no native methods,
 *  so this loads cleanly via DexClassLoader). */
public class SttBridge {
    public static volatile String result = "";
    public static volatile String error = "";
    public static volatile boolean done = false;
    public static volatile boolean listening = false;
    // Diagnostics: did the recognizer actually hear any audio, and how loud?
    public static volatile boolean heard = false;
    public static volatile float maxRms = 0f;
    // onEndOfSpeech fired — the recognizer's VAD thinks you stopped talking. Lets
    // the round-trip fire the LLM on the partial the instant you pause, WITHOUT
    // waiting for the finalize pass or a Stop tap.
    public static volatile boolean speechEnded = false;
    // Latest streaming partial transcript — the round-trip can start the LLM on
    // this the instant you tap Stop, instead of waiting for the recognizer to
    // finalize (which adds ~0.3–0.8s).
    public static volatile String partial = "";
    private static SpeechRecognizer recognizer;

    public static boolean available(Context ctx) {
        try { return SpeechRecognizer.isRecognitionAvailable(ctx); }
        catch (Throwable t) { return false; }
    }

    /** On-device recognizer ONLY — the whole point is zero-network STT for spotty
     *  3G. API 31+ (Android 12) has a dedicated on-device factory that never hits
     *  the network; older devices fall back to the default service with
     *  EXTRA_PREFER_OFFLINE (set in the intent) to keep it local. */
    private static SpeechRecognizer createRecognizer(Context ctx) {
        // createOnDeviceSpeechRecognizer is API 31; isOnDeviceRecognitionAvailable
        // is API 33 — don't call the latter on 31/32 or it throws NoSuchMethodError.
        if (Build.VERSION.SDK_INT >= 31) {
            try {
                Log.i("KairaStt", "createOnDeviceSpeechRecognizer (guaranteed on-device)");
                return SpeechRecognizer.createOnDeviceSpeechRecognizer(ctx);
            } catch (Throwable t) {
                Log.w("KairaStt", "on-device recognizer failed, using default", t);
            }
        }
        Log.i("KairaStt", "default recognizer (offline-preferred)");
        return SpeechRecognizer.createSpeechRecognizer(ctx);
    }

    private static String errText(int e) {
        switch (e) {
            case SpeechRecognizer.ERROR_NETWORK_TIMEOUT: return "network timeout";
            case SpeechRecognizer.ERROR_NETWORK: return "network error";
            case SpeechRecognizer.ERROR_AUDIO: return "audio recording error";
            case SpeechRecognizer.ERROR_SERVER: return "server error";
            case SpeechRecognizer.ERROR_CLIENT: return "client error";
            case SpeechRecognizer.ERROR_SPEECH_TIMEOUT: return "no speech heard";
            case SpeechRecognizer.ERROR_NO_MATCH: return "didn't catch that";
            case SpeechRecognizer.ERROR_RECOGNIZER_BUSY: return "recognizer busy";
            case SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS: return "mic permission denied";
            case SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED: return "language not supported on-device";
            case SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE: return "offline model not installed — download it in Settings › System › Languages › Voice input › offline";
            default: return "recognizer error " + e;
        }
    }

    /** Start listening (on-device). Callable from any thread — hops to the main Looper. */
    public static void start(final Context ctx, final String lang) {
        done = false; result = ""; error = ""; listening = false;
        heard = false; maxRms = 0f; partial = ""; speechEnded = false;
        new Handler(Looper.getMainLooper()).post(new Runnable() {
            public void run() {
                try {
                    if (recognizer != null) { try { recognizer.destroy(); } catch (Throwable t) {} recognizer = null; }
                    recognizer = createRecognizer(ctx);
                    recognizer.setRecognitionListener(new RecognitionListener() {
                        public void onReadyForSpeech(Bundle b) { listening = true; Log.i("KairaStt", "onReadyForSpeech"); }
                        public void onBeginningOfSpeech() { heard = true; Log.i("KairaStt", "onBeginningOfSpeech"); }
                        public void onRmsChanged(float r) { if (r > maxRms) maxRms = r; if (r > 1.5f) heard = true; }
                        public void onBufferReceived(byte[] b) { if (b != null && b.length > 0) heard = true; Log.i("KairaStt", "onBufferReceived " + (b==null?0:b.length)); }
                        public void onEndOfSpeech() { listening = false; speechEnded = true; Log.i("KairaStt", "onEndOfSpeech maxRms=" + maxRms); }
                        public void onPartialResults(Bundle b) {
                            ArrayList<String> l = b.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION);
                            if (l != null && !l.isEmpty() && l.get(0) != null && !l.get(0).isEmpty()) partial = l.get(0);
                            Log.i("KairaStt", "onPartialResults '" + partial + "'");
                        }
                        public void onEvent(int t, Bundle b) {}
                        public void onResults(Bundle b) {
                            ArrayList<String> l = b.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION);
                            String r = (l != null && !l.isEmpty()) ? l.get(0) : "";
                            if ((r == null || r.isEmpty()) && !partial.isEmpty()) r = partial;
                            result = (r == null) ? "" : r;
                            listening = false; done = true;
                            Log.i("KairaStt", "onResults '" + result + "' heard=" + heard + " maxRms=" + maxRms);
                        }
                        public void onError(int e) {
                            Log.i("KairaStt", "onError " + e + " heard=" + heard + " maxRms=" + maxRms + " partial='" + partial + "'");
                            boolean noSpeech = e == SpeechRecognizer.ERROR_NO_MATCH
                                            || e == SpeechRecognizer.ERROR_SPEECH_TIMEOUT;
                            // Salvage a partial transcript if the final result came back empty.
                            if (noSpeech && !partial.isEmpty()) {
                                result = partial;
                            } else {
                                error = errText(e);
                            }
                            listening = false; done = true;
                        }
                    });
                    Intent i = new Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH);
                    i.putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM);
                    i.putExtra(RecognizerIntent.EXTRA_LANGUAGE, lang);
                    i.putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 1);
                    i.putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true);
                    // Do NOT override the silence timeouts — a short value here makes
                    // the recognizer close the mic in ~2s if it hasn't yet detected
                    // speech onset (e.g. the user took a beat to start). Let the
                    // recognizer use its own longer default no-speech window; the
                    // user's Stop tap force-finalizes anyway.
                    // Keep it on-device on older phones too (harmless on API 31+ on-device recognizer).
                    i.putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true);
                    Log.i("KairaStt", "startListening lang=" + lang);
                    recognizer.startListening(i);
                } catch (Throwable ex) {
                    Log.e("KairaStt", "start failed", ex);
                    error = "start failed: " + ex.getMessage(); done = true;
                }
            }
        });
    }

    /** Stop listening → the recognizer finalizes and fires onResults. */
    public static void stop() {
        new Handler(Looper.getMainLooper()).post(new Runnable() {
            public void run() {
                try { if (recognizer != null) recognizer.stopListening(); Log.i("KairaStt", "stopListening"); } catch (Throwable t) {}
            }
        });
    }
}
