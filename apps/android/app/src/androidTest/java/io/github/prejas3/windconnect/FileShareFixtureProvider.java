package io.github.prejas3.windconnect;

import android.content.*;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.*;
import java.util.*;

/** CCS/1 SEC:test APK only, separate UID, Java-only so no target APK runtime dependency. */
public class FileShareFixtureProvider extends ContentProvider {
    @Override public boolean onCreate() { return true; }
    static String fixtureName(String kind, int index) {
        if (kind.equals("images")) return String.format(Locale.ROOT, "photo%02d.jpg", index);
        if (kind.equals("mixed")) return new String[]{"mixed.JPG", "clip.mp4", "paper.pdf", "mystery"}[index];
        if (kind.equals("unsafe")) return "../CON?.jpg";
        return kind + ".jpg";
    }
    private String kind(Uri uri) {
        List<String> parts = uri.getPathSegments();
        if (parts.size() != 2 || !Arrays.asList("images", "mixed", "unknown", "unsafe", "denied", "oversize", "cancel").contains(parts.get(0))) throw new IllegalArgumentException();
        int index = Integer.parseInt(parts.get(1));
        if (index < 0 || index > 19) throw new IllegalArgumentException();
        return parts.get(0);
    }
    private File file(Uri uri) { return new File(getContext().getCacheDir(), "wn40-source/" + kind(uri) + "/" + Integer.parseInt(uri.getLastPathSegment())); }
    @Override public String getType(Uri uri) {
        if (kind(uri).equals("mixed")) return new String[]{"image/jpeg", "video/mp4", "application/pdf", "application/octet-stream"}[Integer.parseInt(uri.getLastPathSegment())];
        return "image/jpeg";
    }
    @Override public Cursor query(Uri uri, String[] projection, String selection, String[] arguments, String sort) {
        String kind = kind(uri);
        MatrixCursor cursor = new MatrixCursor(new String[]{OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE});
        Object size = kind.equals("unknown") ? null : kind.equals("oversize") ? 3L * 1024 * 1024 * 1024 : file(uri).length();
        cursor.addRow(new Object[]{fixtureName(kind, Integer.parseInt(uri.getLastPathSegment())), size});
        return cursor;
    }
    @Override public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!mode.equals("r")) throw new IllegalArgumentException();
        String kind = kind(uri);
        File file = file(uri);
        if (kind.equals("cancel") || kind.equals("images")) {
            try {
                ParcelFileDescriptor[] pipe = ParcelFileDescriptor.createReliablePipe();
                Thread writer = new Thread(() -> {
                    try (OutputStream output = new ParcelFileDescriptor.AutoCloseOutputStream(pipe[1]); InputStream input = new FileInputStream(file)) {
                        byte[] bytes = new byte[kind.equals("cancel") ? 65536 : 256];
                        int count;
                        while ((count = input.read(bytes)) >= 0) { output.write(bytes, 0, count); Thread.sleep(kind.equals("cancel") ? 50 : 20); }
                    } catch (Exception ignored) { }
                });
                writer.setDaemon(true); writer.start(); return pipe[0];
            } catch (IOException failure) { throw new FileNotFoundException(failure.toString()); }
        }
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY);
    }
    @Override public Uri insert(Uri uri, ContentValues values) { throw new UnsupportedOperationException(); }
    @Override public int update(Uri uri, ContentValues values, String selection, String[] arguments) { throw new UnsupportedOperationException(); }
    @Override public int delete(Uri uri, String selection, String[] arguments) { throw new UnsupportedOperationException(); }
}
