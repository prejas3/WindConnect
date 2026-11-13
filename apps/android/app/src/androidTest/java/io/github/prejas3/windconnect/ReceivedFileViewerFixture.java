package io.github.prejas3.windconnect;

import android.app.Activity;
import android.os.Bundle;
import org.json.JSONObject;
import java.io.*;
import java.security.MessageDigest;
import java.util.Locale;

/** CCS/1 CHECK:separate test-APK UID opens the public content URI using only the explicit read grant. */
public class ReceivedFileViewerFixture extends Activity {
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        JSONObject result = new JSONObject();
        try {
            MessageDigest hash = MessageDigest.getInstance("SHA-256"); long size = 0;
            try (InputStream input = getContentResolver().openInputStream(getIntent().getData())) {
                byte[] buffer = new byte[65536]; int n;
                while ((n = input.read(buffer)) >= 0) { hash.update(buffer, 0, n); size += n; }
            }
            StringBuilder hex = new StringBuilder();
            for (byte value : hash.digest()) hex.append(String.format(Locale.ROOT, "%02x", value & 255));
            result.put("uid", android.os.Process.myUid()).put("size", size).put("sha256", hex.toString());
        } catch (Exception error) {
            try { result.put("error", error.toString()); } catch (Exception ignored) { }
        }
        try (FileOutputStream output = new FileOutputStream(new File(getCacheDir(), "wn120-open.json"))) {
            output.write(result.toString().getBytes(java.nio.charset.StandardCharsets.UTF_8));
        } catch (Exception error) { throw new RuntimeException(error); }
        finish();
    }
}
