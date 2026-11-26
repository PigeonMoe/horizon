// Uses Paper 26.2 build 132 and the unmodified HuskSync 4.0.0 at
// c8ab8393868e59ad6f126c34f3f3f8b83ba4feb1 (Item-NBT-API 2.16.0).
// Add patched Paper and its libraries before HuskSync and its runtime jars
// on the classpath. Compile and run with JDK 25 in a scratch directory.
// Arguments: output directory, optional SNBT directory, optional Packed file.
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
import java.util.Set;
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
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.food.FoodData;
import net.william278.husksync.HuskSync;
import net.william278.husksync.adapter.Adaptable;
import net.william278.husksync.adapter.SnappyGsonAdapter;
import net.william278.husksync.config.Settings.SynchronizationSettings.AttributeSettings;
import net.william278.husksync.libraries.desertwell.util.Version;
import net.william278.husksync.libraries.gson.JsonParser;
import net.william278.husksync.libraries.nbtapi.utils.MinecraftVersion;
import org.bukkit.Bukkit;
import org.bukkit.Material;
import org.bukkit.NamespacedKey;
import org.bukkit.attribute.Attribute;
import org.bukkit.attribute.AttributeInstance;
import org.bukkit.attribute.AttributeModifier;
import org.bukkit.craftbukkit.CraftRegistry;
import org.bukkit.craftbukkit.CraftServer;
import org.bukkit.craftbukkit.attribute.CraftAttributeInstance;
import org.bukkit.craftbukkit.potion.CraftPotionUtil;
import org.bukkit.inventory.EquipmentSlotGroup;
import org.bukkit.inventory.ItemStack;
import org.bukkit.persistence.PersistentDataType;
import org.bukkit.potion.PotionEffect;
import org.bukkit.potion.PotionEffectType;
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
        if (args.length < 1 || args.length > 3) {
            throw new IllegalArgumentException("Pass an output directory, optional SNBT directory, and optional Packed file");
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
        if (args.length >= 2) {
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
        if (args.length == 3) {
            var rust = DataSnapshot.deserialize(plugin, Files.readAllBytes(Path.of(args[2])));
            if (rust.isInvalid() || !JsonParser.parseString(rust.asJson(plugin))
                    .equals(JsonParser.parseString(packed.asJson(plugin)))) {
                throw new AssertionError("Rust Packed data changed the Java snapshot");
            }
        }
        var output = Files.createDirectories(Path.of(args[0]));
        for (var field : Map.of("inventory.snbt", bagData, "ender-chest.snbt", chestData,
                "empty-inventory.snbt", emptyBag, "empty-ender-chest.snbt", emptyChest).entrySet()) {
            Files.writeString(output.resolve(field.getKey()), field.getValue());
        }
        Files.write(output.resolve("husk-26.2.snappy"), bytes);
        Files.writeString(output.resolve("husk-26.2.json"), packed.asJson(plugin));

        var food = new FoodData();
        food.setFoodLevel(27);
        food.setSaturation(-0.0f);
        // CraftHumanEntity.setExhaustion writes this field directly.
        food.exhaustionLevel = -3.25f;
        var vitals = new LinkedHashMap<>(data);
        vitals.put(Identifier.HEALTH.toString(), normalizedVital(plugin, BukkitData.Health.class,
                BukkitData.Health.from(73.12345678912345, -0.0, true)));
        vitals.put(Identifier.HUNGER.toString(), normalizedVital(plugin, BukkitData.Hunger.class,
                BukkitData.Hunger.from(food.getFoodLevel(), food.getSaturationLevel(), food.exhaustionLevel)));
        vitals.put(Identifier.EXPERIENCE.toString(), normalizedVital(plugin, BukkitData.Experience.class,
                BukkitData.Experience.from(123456789, 37, 0.23456788f)));
        var precise = new Serializer.Json<>(plugin, BukkitData.Experience.class).deserialize(
                "{\"total_experience\":10,\"exp_level\":3,\"exp_progress\":0.5000000298023223876953125000000001}");
        if (Float.floatToRawIntBits(precise.getExpProgress()) != Float.floatToRawIntBits(0.5f)) {
            throw new AssertionError("HuskSync changed the double-to-float rounding");
        }
        var vitalsPacked = new DataSnapshot.Packed(id, false, savedAt, "API", "fixture", vitals, version, "bukkit", 5);
        Files.writeString(output.resolve("husk-vitals-26.2.json"), vitalsPacked.asJson(plugin));

        var effectsAndAttributes = new LinkedHashMap<>(vitals);
        effectsAndAttributes.put(Identifier.POTION_EFFECTS.toString(), potionEffects(plugin));
        effectsAndAttributes.put(Identifier.ATTRIBUTES.toString(), normalizedVital(
                plugin, BukkitData.Attributes.class, attributes()));
        var effectsPacked = new DataSnapshot.Packed(id, false, savedAt, "API", "fixture",
                effectsAndAttributes, version, "bukkit", 5);
        Files.writeString(output.resolve("husk-effects-attributes-26.2.json"), effectsPacked.asJson(plugin));
    }

    private static String potionEffects(HuskSync plugin) {
        var speed = CraftPotionUtil.fromBukkit(new PotionEffect(PotionEffectType.SPEED,
                -1, 300, false, false, true));
        var regeneration = CraftPotionUtil.fromBukkit(new PotionEffect(PotionEffectType.REGENERATION,
                73, -4, false, true, false));
        var expired = CraftPotionUtil.fromBukkit(new PotionEffect(PotionEffectType.HASTE,
                -19, 4, false, false, false));
        if (speed.getAmplifier() != 255 || !speed.isInfiniteDuration()
                || regeneration.getAmplifier() != 0 || expired.getDuration() != -19) {
            throw new AssertionError("Paper changed native amplifier or duration normalization");
        }
        var ambient = new PotionEffect(PotionEffectType.HASTE, 60, 2, true, true, true);
        var codec = new BukkitSerializer.PotionEffects(plugin);
        var effects = BukkitData.PotionEffects.from(List.of(
                CraftPotionUtil.toBukkit(speed), CraftPotionUtil.toBukkit(regeneration), ambient));
        var result = codec.serialize(effects);
        if (effects.getActiveEffects().size() != 2 || !codec.serialize(codec.deserialize(result)).equals(result)
                || !codec.deserialize("[{\"type\":\"minecraft:haste\",\"amplifier\":2,\"duration\":60,"
                        + "\"is_ambient\":true,\"show_particles\":true,\"has_icon\":true}]")
                        .getActiveEffects().isEmpty()) {
            throw new AssertionError("HuskSync changed its application flags or ambient effect policy");
        }
        return result;
    }

    private static BukkitData.Attributes attributes() throws Exception {
        var health = new CraftAttributeInstance(new net.minecraft.world.entity.ai.attributes.AttributeInstance(
                Attributes.MAX_HEALTH, unused -> {}), Attribute.MAX_HEALTH);
        health.setBaseValue(1200.125);
        health.addModifier(attributeModifier("health", 5.25, AttributeModifier.Operation.ADD_NUMBER,
                EquipmentSlotGroup.ANY));
        health.addModifier(new AttributeModifier(new NamespacedKey("minecraft", "effect.fixture"), 1,
                AttributeModifier.Operation.ADD_NUMBER, EquipmentSlotGroup.ANY));
        var damage = new CraftAttributeInstance(new net.minecraft.world.entity.ai.attributes.AttributeInstance(
                Attributes.ATTACK_DAMAGE, unused -> {}), Attribute.ATTACK_DAMAGE);
        damage.setBaseValue(7.125);
        damage.addModifier(attributeModifier("damage", 0.125, AttributeModifier.Operation.ADD_SCALAR,
                EquipmentSlotGroup.ANY));

        var settingsConstructor = AttributeSettings.class.getDeclaredConstructor();
        settingsConstructor.setAccessible(true);
        var settings = settingsConstructor.newInstance();
        var ignored = AttributeSettings.class.getDeclaredField("ignoredModifiers");
        ignored.setAccessible(true);
        ignored.set(settings, List.of("minecraft:effect.*", "minecraft:creative_mode_*", "pmattibute:*"));
        var capture = BukkitData.Attributes.class.getDeclaredMethod("adapt", AttributeInstance.class, AttributeSettings.class);
        capture.setAccessible(true);
        var healthData = (Data.Attributes.Attribute) capture.invoke(null, health, settings);
        var damageData = (Data.Attributes.Attribute) capture.invoke(null, damage, settings);
        if (healthData.baseValue() != 1200.125 || healthData.modifiers().size() != 1) {
            throw new AssertionError("Unexpected HuskSync base or modifier fixture: "
                    + healthData.baseValue() + ", " + healthData.modifiers().stream().map(Data.Attributes.Modifier::name).toList());
        }

        // Permanent NMS modifiers lose equipment slot groups. Exercise the
        // original modifier adapter directly for a slot-bearing Bukkit value.
        var captureModifier = BukkitData.Attributes.class.getDeclaredMethod("adapt", AttributeModifier.class);
        captureModifier.setAccessible(true);
        var speedModifier = (Data.Attributes.Modifier) captureModifier.invoke(null,
                attributeModifier("speed", -0.0, AttributeModifier.Operation.MULTIPLY_SCALAR_1,
                        EquipmentSlotGroup.MAINHAND));
        var speed = new Data.Attributes.Attribute(Attribute.MOVEMENT_SPEED.getKey().toString(),
                0.12012345678912345, Set.of(speedModifier));
        var constructor = BukkitData.Attributes.class.getDeclaredConstructor(List.class);
        constructor.setAccessible(true);
        return constructor.newInstance(List.of(healthData, damageData, speed));
    }

    private static AttributeModifier attributeModifier(String name, double amount,
            AttributeModifier.Operation operation, EquipmentSlotGroup group) {
        return new AttributeModifier(new NamespacedKey("oracle", name), amount, operation, group);
    }

    private static <T extends Data & Adaptable> String normalizedVital(HuskSync plugin, Class<T> type, T value) {
        var codec = new Serializer.Json<>(plugin, type);
        String json = codec.serialize(value);
        if (!codec.serialize(codec.deserialize(json)).equals(json)) {
            throw new AssertionError("HuskSync changed the normalized " + type.getSimpleName() + " fields");
        }
        return json;
    }

    private static void assertEquivalent(String java, String rust) throws Exception {
        if (!TagParser.parseCompoundFully(java).equals(TagParser.parseCompoundFully(rust))) {
            throw new AssertionError("Rust SNBT differs from the source Java field");
        }
    }
}
