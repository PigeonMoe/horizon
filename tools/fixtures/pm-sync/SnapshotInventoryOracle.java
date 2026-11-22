// Uses Paper 26.2 build 132 and the unmodified HuskSync 4.0.0 at
// c8ab8393868e59ad6f126c34f3f3f8b83ba4feb1 (Item-NBT-API 2.16.0).
// Add patched Paper and its libraries before HuskSync and its runtime jars
// on the classpath. Compile and run with JDK 25 in a scratch directory.
// Arguments: output directory, optional directory of SNBT files to verify.
package net.william278.husksync.data;

import java.lang.reflect.Proxy;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.OffsetDateTime;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.logging.Logger;
import net.kyori.adventure.text.Component;
import net.minecraft.SharedConstants;
import net.minecraft.commands.Commands;
import net.minecraft.nbt.TagParser;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.dedicated.DedicatedServer;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.util.Util;
import net.minecraft.world.level.WorldDataConfiguration;
import net.william278.husksync.HuskSync;
import net.william278.husksync.adapter.SnappyGsonAdapter;
import net.william278.husksync.libraries.desertwell.util.Version;
import net.william278.husksync.libraries.nbtapi.utils.MinecraftVersion;
import org.bukkit.Bukkit;
import org.bukkit.Material;
import org.bukkit.NamespacedKey;
import org.bukkit.craftbukkit.CraftRegistry;
import org.bukkit.craftbukkit.CraftServer;
import org.bukkit.inventory.ItemStack;
import org.bukkit.persistence.PersistentDataType;
import sun.misc.Unsafe;

public final class SnapshotInventoryOracle {
    private static void setField(Class<?> owner, Object receiver, String name, Object value) throws Exception {
        Field field = owner.getDeclaredField(name);
        field.setAccessible(true);
        field.set(receiver, value);
    }

    private static void bootstrapPaper() throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var packs = new WorldLoader.PackConfig(ServerPacksSource.createVanillaTrustedRepository(),
                WorldDataConfiguration.DEFAULT, false, true);
        var loading = new WorldLoader.InitConfig(packs, Commands.CommandSelection.DEDICATED,
                LevelBasedPermissionSet.ALL_PERMISSIONS);
        var registries = WorldLoader.load(loading,
                context -> new WorldLoader.DataLoadOutput<>(null, context.datapackDimensions()),
                (resources, datapack, layers, unused) -> layers,
                Util.backgroundExecutor(), Runnable::run).join();
        CraftRegistry.setMinecraftRegistry(registries.compositeAccess());
        // NBT API looks up CraftServer.getServer().registryAccess() by reflection.
        // Populate only this state for the oracle, without a world or server listener.
        var unsafeField = Unsafe.class.getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        var unsafe = (Unsafe) unsafeField.get(null);
        var vanilla = (DedicatedServer) unsafe.allocateInstance(DedicatedServer.class);
        setField(MinecraftServer.class, vanilla, "registries", registries);
        var paper = (CraftServer) unsafe.allocateInstance(CraftServer.class);
        setField(CraftServer.class, paper, "console", vanilla);
        setField(CraftServer.class, paper, "serverVersion", "Paper-fixture-26.2");
        setField(CraftServer.class, paper, "bukkitVersion", "26.2-R0.1-SNAPSHOT");
        setField(CraftServer.class, paper, "logger", Logger.getLogger("HuskSync inventory fixture"));
        Bukkit.setServer(paper);
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 1 || args.length > 2) {
            throw new IllegalArgumentException("Pass an output directory and an optional verification directory");
        }
        MinecraftVersion.disableBStats();
        MinecraftVersion.disableUpdateCheck();
        bootstrapPaper();
        var version = Version.fromString("26.2");
        var plugin = (HuskSync) Proxy.newProxyInstance(HuskSync.class.getClassLoader(),
                new Class<?>[] { HuskSync.class }, (proxy, method, callArgs) -> switch (method.getName()) {
                    case "getGson" -> ((HuskSync) proxy).createGson();
                    case "getDataAdapter" -> new SnappyGsonAdapter((HuskSync) proxy);
                    case "getMinecraftVersion" -> version;
                    case "getPlatformType" -> "bukkit";
                    default -> {
                        if (method.isDefault()) yield java.lang.reflect.InvocationHandler.invokeDefault(proxy, method, callArgs);
                        throw new UnsupportedOperationException(method.toString());
                    }
                });
        var inventory = new BukkitSerializer.Inventory(plugin);
        var ender = new BukkitSerializer.EnderChest(plugin);
        var gear = new ItemStack[41];
        gear[4] = new ItemStack(Material.DIAMOND, 3);
        gear[4].editMeta(meta -> {
            meta.displayName(Component.text("原有名称"));
            meta.lore(List.of(Component.text("原有描述"), Component.text("旧物品 🙂")));
            meta.getPersistentDataContainer().set(new NamespacedKey("oracle", "original"),
                    PersistentDataType.STRING, "保留");
        });
        gear[32] = new ItemStack(Material.TORCH, 17);
        gear[36] = new ItemStack(Material.IRON_BOOTS);
        gear[39] = new ItemStack(Material.IRON_HELMET);
        gear[40] = new ItemStack(Material.SHIELD);
        var chest = new ItemStack[27];
        chest[26] = new ItemStack(Material.GOLD_INGOT, 11);
        String bagData = inventory.serialize(BukkitData.Items.Inventory.from(gear, 4));
        String chestData = ender.serialize(BukkitData.Items.EnderChest.adapt(chest));
        String emptyBag = inventory.serialize(BukkitData.Items.Inventory.empty());
        String emptyChest = ender.serialize(BukkitData.Items.EnderChest.empty());
        if (args.length == 2) {
            var verify = Path.of(args[1]);
            assertEquivalent(bagData, Files.readString(verify.resolve("inventory.snbt")));
            assertEquivalent(chestData, Files.readString(verify.resolve("ender-chest.snbt")));
            assertEquivalent(emptyBag, Files.readString(verify.resolve("empty-inventory.snbt")));
            assertEquivalent(emptyChest, Files.readString(verify.resolve("empty-ender-chest.snbt")));
        }
        var parsedBag = inventory.deserialize(bagData, version);
        var parsedChest = ender.deserialize(chestData, version);
        if (!Arrays.equals(gear, parsedBag.getContents()) || parsedBag.getHeldItemSlot() != 4
                || !Arrays.equals(chest, parsedChest.getContents())) throw new AssertionError("Bukkit codec changed slots/items");
        var data = new LinkedHashMap<String, String>();
        data.put(Identifier.INVENTORY.toString(), bagData);
        data.put(Identifier.ENDER_CHEST.toString(), chestData);
        var id = UUID.fromString("60093724-28b0-4769-a72b-382a9f922d35");
        var savedAt = OffsetDateTime.parse("2026-11-23T00:00:00Z");
        var packed = new DataSnapshot.Packed(id, false, savedAt, "API", "fixture", data, version, "bukkit", 5);
        byte[] bytes = packed.asBytes(plugin);
        var restored = DataSnapshot.deserialize(plugin, bytes);
        if (restored.isInvalid() || !restored.asJson(plugin).equals(packed.asJson(plugin))) {
            throw new AssertionError("HuskSync Packed encoding changed snapshot");
        }
        var newer = new DataSnapshot.Packed(id, false, savedAt, "API", "fixture", data,
                Version.fromString("26.3"), "bukkit", 5);
        if (!DataSnapshot.deserialize(plugin, newer.asBytes(plugin)).isInvalid()) {
            throw new AssertionError("Paper 26.2 accepted a snapshot declaring 26.3");
        }
        var output = Files.createDirectories(Path.of(args[0]));
        for (var field : Map.of("inventory.snbt", bagData, "ender-chest.snbt", chestData,
                "empty-inventory.snbt", emptyBag, "empty-ender-chest.snbt", emptyChest).entrySet()) {
            Files.writeString(output.resolve(field.getKey()), field.getValue());
        }
        Files.write(output.resolve("husk-26.2.snappy"), bytes);
        Files.writeString(output.resolve("husk-26.2.json"), packed.asJson(plugin));
    }

    private static void assertEquivalent(String java, String rust) throws Exception {
        if (!TagParser.parseCompoundFully(java).equals(TagParser.parseCompoundFully(rust))) {
            throw new AssertionError("Rust SNBT differs from the source Java field");
        }
    }
}
