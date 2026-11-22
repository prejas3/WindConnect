package io.github.prejas3.windconnect;

import android.app.Activity;
import android.content.*;
import android.graphics.*;
import android.net.Uri;
import android.os.Bundle;
import android.provider.MediaStore;
import org.json.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.util.*;

/** CCS/1 CHECK:standalone external-UID share source; generated data only. */
public class FileShareSourceActivity extends Activity {
    private static String hash(byte[] bytes) throws Exception {
        byte[] digest = MessageDigest.getInstance("SHA-256").digest(bytes);
        StringBuilder result = new StringBuilder();
        for (byte value : digest) result.append(String.format(Locale.ROOT, "%02x", value & 255));
        return result.toString();
    }
    private void writeManifest(String kind, JSONArray rows) throws IOException {
        try (OutputStream output = new FileOutputStream(new File(getCacheDir(), "wn40-" + kind + ".json"))) { output.write(rows.toString().getBytes(StandardCharsets.UTF_8)); }
    }
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        try { prepare(); finish(); } catch (Exception failure) { throw new RuntimeException(failure); }
    }
    private void prepare() throws Exception {
        String kind = getIntent().getStringExtra("case");
        if (kind == null) kind = "images";
        if (kind.equals("cleanup")) {
            String selected = getSharedPreferences("fixture", MODE_PRIVATE).getString("picker", null);
            if (selected != null) getContentResolver().delete(Uri.parse(selected), null, null);
            return;
        }
        if (kind.equals("picker")) {
            ContentValues values = new ContentValues();
            values.put(MediaStore.MediaColumns.DISPLAY_NAME, "wn40-picker.pdf");
            values.put(MediaStore.MediaColumns.MIME_TYPE, "application/pdf");
            values.put(MediaStore.MediaColumns.RELATIVE_PATH, "Download/WN40Fixture");
            Uri uri = Objects.requireNonNull(getContentResolver().insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values));
            byte[] bytes = "%PDF-1.4\nWN40 PICKER\n%%EOF".getBytes(StandardCharsets.UTF_8);
            try (OutputStream output = getContentResolver().openOutputStream(uri)) { output.write(bytes); }
            writeManifest(kind, new JSONArray().put(new JSONObject().put("name", "wn40-picker.pdf").put("size", bytes.length).put("sha256", hash(bytes))));
            getSharedPreferences("fixture", MODE_PRIVATE).edit().putString("picker", uri.toString()).apply();
            return;
        }
        if (!Arrays.asList("images", "mixed", "unknown", "unsafe", "denied", "oversize", "cancel", "retry").contains(kind)) throw new IllegalArgumentException();
        if (kind.equals("retry")) { new File(getCacheDir(), "wn41-failed-once").delete(); new File(getCacheDir(), "wn41-first-two-done").delete(); }
        int count = kind.equals("images") ? 20 : kind.equals("mixed") ? 4 : kind.equals("retry") ? 5 : 1;
        File directory = new File(getCacheDir(), "wn40-source/" + kind); directory.mkdirs();
        ArrayList<Uri> uris = new ArrayList<>(); JSONArray manifest = new JSONArray();
        for (int index = 0; index < count; index++) {
            File file = new File(directory, Integer.toString(index));
            if (kind.equals("retry")) {
                Files.write(file.toPath(), ("WN41 retry file " + index).getBytes(StandardCharsets.UTF_8));
            } else if (kind.equals("cancel")) {
                byte[] bytes = new byte[65536]; Arrays.fill(bytes, (byte)42);
                try (OutputStream output = new FileOutputStream(file)) { for (int i = 0; i < 160; i++) output.write(bytes); }
            } else if (kind.equals("mixed") && index > 0) {
                Files.write(file.toPath(), ("WN40 GENERATED " + kind + " " + index).getBytes(StandardCharsets.UTF_8));
            } else {
                Bitmap image = Bitmap.createBitmap(160, 120, Bitmap.Config.ARGB_8888); Canvas canvas = new Canvas(image);
                canvas.drawColor(Color.rgb(20 + index * 9, 80, 180 - index * 6)); Paint paint = new Paint(); paint.setColor(Color.WHITE);
                canvas.drawCircle(20 + index * 5, 60, 18, paint);
                try (OutputStream output = new FileOutputStream(file)) { image.compress(Bitmap.CompressFormat.JPEG, 90, output); }
                image.recycle();
            }
            String name = kind.equals("unsafe") ? "_CON_.jpg" : FileShareFixtureProvider.fixtureName(kind, index);
            manifest.put(new JSONObject().put("name", name).put("size", file.length()).put("sha256", hash(Files.readAllBytes(file.toPath()))));
            uris.add(Uri.parse("content://" + getPackageName() + ".files_fixture/" + kind + "/" + index));
        }
        writeManifest(kind, manifest);
        Intent send = new Intent(Intent.ACTION_SEND_MULTIPLE).setType(kind.equals("mixed") ? "*/*" : "image/jpeg").putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris);
        ClipData clip = ClipData.newRawUri("Synthetic WN40 files", uris.get(0));
        for (int index = 1; index < uris.size(); index++) clip.addItem(new ClipData.Item(uris.get(index)));
        send.setClipData(clip);
        if (!kind.equals("denied")) send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        if (getIntent().getBooleanExtra("chooser", true)) startActivity(Intent.createChooser(send, "Share files WN40"));
        else startActivity(send.setClassName("io.github.prejas3.windconnect", "io.github.prejas3.windconnect.ShareFilesActivity"));
    }
}
