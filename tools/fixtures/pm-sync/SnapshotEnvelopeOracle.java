package moe.pigeon.pmsync;

import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.HexFormat;
import java.util.Map;
import java.util.Set;

// Compile alongside the original PM-Sync SnapshotEnvelope.java at
// d1d1c74866e0104524b0015c65c9853f320e467d, then run to regenerate
// crates/horizon-server-files/tests/fixtures/pm-sync/java-v2.hex.
public final class SnapshotEnvelopeOracle {
    private static final byte[] HUSK = { 0x1f, (byte) 0x8b, 0, (byte) 0xff, 0x0a };

    private static void emit(String name, byte[] bytes) {
        System.out.println(name + " " + HexFormat.of().formatHex(bytes));
    }

    private static byte[] sign(byte[] body) throws Exception {
        var signed = new ByteArrayOutputStream();
        signed.write(body);
        signed.write(MessageDigest.getInstance("SHA-256").digest(body));
        return signed.toByteArray();
    }

    private static void verify(byte[] bytes) throws IOException {
        byte[] encoded = SnapshotEnvelope.decode(bytes, bytes.length).encode();
        if (!Arrays.equals(bytes, encoded)) throw new AssertionError("Canonical round trip changed bytes");
        reject(bytes, bytes.length - 1);
    }

    private static void reject(byte[] bytes, int limit) throws IOException {
        try {
            SnapshotEnvelope.decode(bytes, limit);
        } catch (IOException expected) {
            return;
        }
        throw new AssertionError("Invalid snapshot was accepted");
    }

    public static void main(String[] args) throws Exception {
        var recipes = Set.of("", "pm:中文\0", "pm:\ud83d\ude42", "pm:\ue000", "\ud800");
        var sections = Map.of("slimefun", new byte[] { 'S', 'F', 'P', '1', 0, (byte) 0xff },
                              "addon_z-9", new byte[] { 0, 1, (byte) 0x80 });
        byte[] canonical = new SnapshotEnvelope(73.25, recipes, sections, HUSK).encode();
        verify(canonical);
        emit("canonical", canonical);

        byte[] empty = new SnapshotEnvelope(-0.0, Set.of(), Map.of(), new byte[] { 0 }).encode();
        verify(empty);
        emit("empty", empty);

        var body = new ByteArrayOutputStream();
        try (var out = new DataOutputStream(body)) {
            out.writeInt(0x504d5302);
            out.writeDouble(73.25);
            out.writeInt(7);
            out.writeUTF("pm:\ue000");
            out.writeUTF("pm:\ud83d\ude42");
            out.writeUTF("\ud800");
            out.writeUTF("");
            out.writeUTF("pm:中文\0");
            // DataInputStream also accepts overlong modified UTF and a literal NUL.
            out.writeShort(11);
            out.write(new byte[] { (byte) 0xc1, (byte) 0xb0, 'm', ':', (byte) 0xe4, (byte) 0xb8,
                                   (byte) 0xad, (byte) 0xe6, (byte) 0x96, (byte) 0x87, 0 });
            out.writeShort(10);
            out.write(new byte[] { 'p', 'm', ':', (byte) 0xe4, (byte) 0xb8, (byte) 0xad,
                                   (byte) 0xe6, (byte) 0x96, (byte) 0x87, 0 });
            out.writeInt(2);
            for (String name : new String[] { "slimefun", "addon_z-9" }) {
                out.writeUTF(name);
                out.writeInt(sections.get(name).length);
                out.write(sections.get(name));
            }
            out.writeInt(HUSK.length);
            out.write(HUSK);
        }
        byte[] noncanonical = sign(body.toByteArray());
        byte[] canonicalized = SnapshotEnvelope.decode(noncanonical, noncanonical.length).encode();
        if (!Arrays.equals(canonical, canonicalized)) throw new AssertionError("Noncanonical round trip changed values");
        emit("noncanonical", noncanonical);

        body.write(0x5a);
        byte[] trailing = sign(body.toByteArray());
        reject(trailing, trailing.length);
        emit("trailing", trailing);
    }
}
