// Golden image harness for the Rust port of ShareX.ImageEditor's image effects.
// Runs only inside a Docker container (never on the host). It compiles upstream's
// Core/ImageEffects sources as they are, writes:
//   /out/manifest.json  every effect with its metadata and parameter schema
//   /out/cases.json     every rendered case (effect, input, parameter values)
//   /out/inputs/*.png   the fixed inputs
//   /out/expected/*.png the reference outputs

using System.Diagnostics;
using System.Globalization;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Nodes;
using ShareX.ImageEditor.Core.ImageEffects;
using ShareX.ImageEditor.Core.ImageEffects.Parameters;
using SkiaSharp;

static class Program
{
    const int W = 96;
    const int H = 64;

    static int Main(string[] args)
    {
        CultureInfo.DefaultThreadCurrentCulture = CultureInfo.InvariantCulture;
        string outDir = args.Length > 0 ? args[0] : "/out";
        string filter = args.Length > 1 ? args[1] : "";
        Directory.CreateDirectory(Path.Combine(outDir, "inputs"));
        Directory.CreateDirectory(Path.Combine(outDir, "expected"));

        Save(MakePhoto(), Path.Combine(outDir, "inputs", "photo.png"));
        Save(MakeAlpha(Load(Path.Combine(outDir, "inputs", "photo.png"))), Path.Combine(outDir, "inputs", "alpha.png"));
        var inputs = new Dictionary<string, SKBitmap>
        {
            ["photo"] = Load(Path.Combine(outDir, "inputs", "photo.png")),
            ["alpha"] = Load(Path.Combine(outDir, "inputs", "alpha.png")),
        };

        List<ImageEffectBase> effects = typeof(ImageEffectBase).Assembly.GetTypes()
            .Where(t => t is { IsAbstract: false, IsClass: true } && typeof(ImageEffectBase).IsAssignableFrom(t) && t.GetConstructor(Type.EmptyTypes) != null)
            .Select(t => (ImageEffectBase)Activator.CreateInstance(t)!)
            .OrderBy(e => e.Category)
            .ThenBy(e => e.Name, StringComparer.OrdinalIgnoreCase)
            .ToList();

        var manifest = new JsonArray();
        var cases = new JsonArray();
        string[] only = filter.Split(',', StringSplitOptions.RemoveEmptyEntries);
        foreach (ImageEffectBase e in effects)
        {
            var meta = new JsonObject
            {
                ["id"] = e.Id,
                ["name"] = e.Name,
                ["type"] = e.GetType().Name,
                ["category"] = e.Category.ToString(),
                ["icon"] = string.Concat(e.IconKey.Select(ch => ((int)ch).ToString("x4"))),
                ["description"] = e.Description,
                ["browser_label"] = e.BrowserLabel,
                ["mode"] = e.ExecutionMode.ToString(),
            };
            var ps = new JsonArray();
            foreach (EffectParameter p in e.Parameters) ps.Add(Describe(p));
            meta["parameters"] = ps;
            manifest.Add(meta);

            if (only.Length > 0 && !only.Contains(e.Id)) continue;

            var sets = new List<(string Set, string Input, Dictionary<string, object?> Values)>
            {
                ("default", "photo", Defaults(e)),
                ("default", "alpha", Defaults(e)),
            };
            if (e.Parameters.Count > 0) sets.Add(("alt", "photo", Alt(e)));

            foreach (var (set, input, values) in sets)
            {
                string name = $"{e.Id}.{set}.{input}";
                var c = new JsonObject
                {
                    ["name"] = name,
                    ["effect"] = e.Id,
                    ["set"] = set,
                    ["input"] = input,
                    ["values"] = ValuesJson(e, values),
                };
                try
                {
                    var sw = Stopwatch.StartNew();
                    using SKBitmap r1 = Run(e.GetType(), values, inputs[input]);
                    sw.Stop();
                    using SKBitmap r2 = Run(e.GetType(), values, inputs[input]);
                    c["deterministic"] = Same(r1, r2);
                    c["width"] = r1.Width;
                    c["height"] = r1.Height;
                    c["ms"] = sw.ElapsedMilliseconds;
                    Save(r1, Path.Combine(outDir, "expected", name + ".png"));
                }
                catch (Exception ex)
                {
                    c["error"] = ex.GetType().Name + ": " + ex.Message;
                }
                cases.Add(c);
                Console.WriteLine(name + (c["error"] != null ? "  ERROR " + c["error"] : ""));
            }
        }

        var options = new JsonSerializerOptions { WriteIndented = true, Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping };
        File.WriteAllText(Path.Combine(outDir, "manifest.json"), manifest.ToJsonString(options));
        File.WriteAllText(Path.Combine(outDir, "cases.json"), cases.ToJsonString(options));
        Console.WriteLine($"{effects.Count} effects, {cases.Count} cases");
        return 0;
    }

    static SKBitmap Run(Type type, Dictionary<string, object?> values, SKBitmap input)
    {
        var effect = (ImageEffectBase)Activator.CreateInstance(type)!;
        foreach (EffectParameter p in effect.Parameters)
        {
            values.TryGetValue(p.Key, out object? v);
            p.Apply(effect, v);
        }
        using SKBitmap copy = input.Copy();
        if (effect is ShareX.ImageEditor.Core.ImageEffects.Adjustments.GammaImageEffect gamma)
        {
            // SkiaSharp 3 rejects the null alpha table GammaImageEffect passes
            // (ArgumentNullException); the port applies the table as intended, with the
            // alpha channel left alone. Same table, same drawing as the effect.
            byte[] table = new byte[256];
            for (int i = 0; i < 256; i++)
            {
                float val = i / 255f;
                float corrected = (float)Math.Pow(val, 1.0 / gamma.Amount);
                table[i] = (byte)(Math.Max(0, Math.Min(1, corrected)) * 255);
            }
            byte[] identity = Enumerable.Range(0, 256).Select(i => (byte)i).ToArray();
            using SKColorFilter filter = SKColorFilter.CreateTable(identity, table, table, table);
            var gammaResult = new SKBitmap(copy.Width, copy.Height, copy.ColorType, copy.AlphaType);
            using var gammaCanvas = new SKCanvas(gammaResult);
            gammaCanvas.Clear(SKColors.Transparent);
            using var gammaPaint = new SKPaint { ColorFilter = filter };
            gammaCanvas.DrawBitmap(copy, 0, 0, gammaPaint);
            return gammaResult;
        }
        SKBitmap result = effect.Apply(copy, new ImageEffectContext(false));
        if (ReferenceEquals(result, copy)) return input.Copy();
        return result;
    }

    static bool Same(SKBitmap a, SKBitmap b)
    {
        if (a.Width != b.Width || a.Height != b.Height) return false;
        return a.Pixels.SequenceEqual(b.Pixels);
    }

    // ---------- parameters ----------

    static string? NumberKind(EffectParameter p)
    {
        // IntSlider / FloatSlider and IntNumeric / DoubleNumeric differ only in the closure
        // they wrap: find the captured Action<TEffect, T> and read T.
        FieldInfo? f = p.GetType().GetField("_applyValue", BindingFlags.NonPublic | BindingFlags.Instance);
        if (f?.GetValue(p) is not Delegate d || d.Target == null) return null;
        foreach (FieldInfo cf in d.Target.GetType().GetFields(BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance))
        {
            if (cf.FieldType.IsGenericType && cf.FieldType.GetGenericTypeDefinition() == typeof(Action<,>))
            {
                Type t = cf.FieldType.GetGenericArguments()[1];
                return t == typeof(int) ? "int" : t == typeof(float) ? "float" : t == typeof(double) ? "double" : t.Name;
            }
        }
        return null;
    }

    static JsonObject Describe(EffectParameter p)
    {
        var o = new JsonObject { ["key"] = p.Key, ["label"] = p.Label };
        if (p.Description != null) o["description"] = p.Description;
        switch (p)
        {
            case SliderEffectParameter s:
                o["kind"] = "slider";
                o["number"] = NumberKind(p);
                o["min"] = s.Minimum; o["max"] = s.Maximum; o["default"] = s.DefaultValue;
                o["tick"] = s.TickFrequency; o["snap"] = s.IsSnapToTickEnabled; o["format"] = s.ValueStringFormat;
                break;
            case CheckboxEffectParameter c:
                o["kind"] = "checkbox"; o["default"] = c.DefaultValue;
                break;
            case EnumEffectParameter en:
                o["kind"] = "enum";
                var opts = new JsonArray();
                foreach (EffectOption op in en.Options) opts.Add(new JsonObject { ["label"] = op.Label, ["value"] = op.Value.ToString() });
                o["options"] = opts; o["default_index"] = en.DefaultIndex;
                break;
            case ColorEffectParameter col:
                o["kind"] = "color"; o["default"] = Hex(col.DefaultValue);
                break;
            case NumericEffectParameter n:
                o["kind"] = "numeric";
                o["number"] = NumberKind(p);
                o["min"] = (double)n.Minimum; o["max"] = (double)n.Maximum; o["default"] = (double)n.DefaultValue;
                o["increment"] = (double)n.Increment; o["format"] = n.FormatString;
                break;
            case TextEffectParameter t:
                o["kind"] = "text"; o["default"] = t.DefaultValue;
                break;
            case FilePathEffectParameter fp:
                o["kind"] = "file"; o["default"] = fp.DefaultValue; o["filter"] = fp.FileFilter;
                break;
            default:
                o["kind"] = p.GetType().Name;
                break;
        }
        return o;
    }

    static Dictionary<string, object?> Defaults(ImageEffectBase e)
    {
        var d = new Dictionary<string, object?>();
        foreach (EffectParameter p in e.Parameters)
        {
            d[p.Key] = p switch
            {
                SliderEffectParameter s => s.DefaultValue,
                CheckboxEffectParameter c => c.DefaultValue,
                EnumEffectParameter en => en.Options[en.DefaultIndex].Value,
                ColorEffectParameter col => col.DefaultValue,
                NumericEffectParameter n => (double)n.DefaultValue,
                TextEffectParameter t => t.DefaultValue,
                FilePathEffectParameter fp => fp.DefaultValue,
                _ => null,
            };
        }
        return d;
    }

    static double Between(double min, double max, double def)
    {
        double v = def < max ? def + (max - def) / 2 : def - (def - min) / 2;
        if (max - min >= 20) return Math.Floor(v);
        return Math.Round(v, 2);
    }

    static Dictionary<string, object?> Alt(ImageEffectBase e)
    {
        var d = Defaults(e);
        foreach (EffectParameter p in e.Parameters)
        {
            switch (p)
            {
                case SliderEffectParameter s:
                    d[p.Key] = Between(s.Minimum, s.Maximum, s.DefaultValue);
                    break;
                case NumericEffectParameter n:
                    {
                        double v = Between((double)n.Minimum, (double)n.Maximum, (double)n.DefaultValue);
                        if (n.Increment >= 1) v = Math.Floor(v);
                        d[p.Key] = v;
                        break;
                    }
                case CheckboxEffectParameter c:
                    d[p.Key] = !c.DefaultValue;
                    break;
                case EnumEffectParameter en:
                    d[p.Key] = en.Options[(en.DefaultIndex + 1) % en.Options.Count].Value;
                    break;
                case ColorEffectParameter col:
                    d[p.Key] = col.DefaultValue == new SKColor(32, 128, 224) ? new SKColor(224, 64, 32) : new SKColor(32, 128, 224);
                    break;
            }
        }
        return d;
    }

    static JsonObject ValuesJson(ImageEffectBase e, Dictionary<string, object?> values)
    {
        var o = new JsonObject();
        foreach (EffectParameter p in e.Parameters)
        {
            object? v = values.GetValueOrDefault(p.Key);
            o[p.Key] = v switch
            {
                null => null,
                bool b => b,
                double x => x,
                SKColor c => Hex(c),
                string s => s,
                _ => v.ToString(),
            };
        }
        return o;
    }

    static string Hex(SKColor c) => $"#{c.Alpha:X2}{c.Red:X2}{c.Green:X2}{c.Blue:X2}";

    // ---------- inputs ----------

    static float Hash(int x, int y, int seed)
    {
        unchecked
        {
            uint h = 2166136261u;
            h = (h ^ (uint)x) * 16777619u;
            h = (h ^ (uint)y) * 16777619u;
            h = (h ^ (uint)seed) * 16777619u;
            h ^= h >> 13;
            h *= 1274126177u;
            h ^= h >> 16;
            return (h & 0x00FFFFFFu) / 16777215f;
        }
    }

    static SKBitmap MakePhoto()
    {
        var bmp = new SKBitmap(new SKImageInfo(W, H, SKColorType.Bgra8888, SKAlphaType.Premul));
        using (var canvas = new SKCanvas(bmp))
        {
            using var bg = new SKPaint
            {
                Shader = SKShader.CreateLinearGradient(new SKPoint(0, 0), new SKPoint(W, H),
                    [new SKColor(28, 64, 128), new SKColor(120, 150, 200), new SKColor(236, 196, 88)], [0f, 0.5f, 1f], SKShaderTileMode.Clamp)
            };
            canvas.DrawRect(0, 0, W, H, bg);
            using var p = new SKPaint { IsAntialias = true };
            p.Color = new SKColor(220, 40, 50);
            canvas.DrawCircle(26, 22, 14, p);
            p.Color = new SKColor(40, 170, 90);
            canvas.DrawRoundRect(new SKRect(50, 8, 86, 32), 6, 6, p);
            p.Color = SKColors.White;
            canvas.DrawRect(6, 42, 30, 16, p);
            p.Color = new SKColor(250, 220, 60);
            canvas.DrawCircle(66, 22, 6, p);
            p.Color = SKColors.Black;
            p.Style = SKPaintStyle.Stroke;
            p.StrokeWidth = 2;
            canvas.DrawLine(40, 60, 90, 38, p);
            p.Style = SKPaintStyle.Fill;
            p.IsAntialias = false;
            for (int x = 56; x < 92; x += 4)
            {
                p.Color = new SKColor(245, 245, 245);
                canvas.DrawRect(x, 46, 2, 14, p);
                p.Color = new SKColor(25, 25, 30);
                canvas.DrawRect(x + 2, 46, 2, 14, p);
            }
        }
        SKColor[] px = bmp.Pixels;
        for (int y = 0; y < H; y++)
        {
            for (int x = 0; x < W; x++)
            {
                int i = y * W + x;
                SKColor c = px[i];
                int n = (int)MathF.Round((Hash(x, y, 7) - 0.5f) * 10f);
                px[i] = new SKColor(Clamp(c.Red + n), Clamp(c.Green + n), Clamp(c.Blue + n), 255);
            }
        }
        bmp.Pixels = px;
        return bmp;
    }

    static SKBitmap MakeAlpha(SKBitmap photo)
    {
        var bmp = new SKBitmap(new SKImageInfo(W, H, SKColorType.Bgra8888, SKAlphaType.Premul));
        SKColor[] px = photo.Pixels;
        for (int y = 0; y < H; y++)
        {
            for (int x = 0; x < W; x++)
            {
                int i = y * W + x;
                float dx = (x + 0.5f - W / 2f) / (W / 2f);
                float dy = (y + 0.5f - H / 2f) / (H / 2f);
                float d = MathF.Sqrt(dx * dx + dy * dy);
                float a = d <= 0.55f ? 255f : MathF.Max(40f, 255f - (d - 0.55f) / 0.45f * 215f);
                if (x < 4 || y < 4 || x >= W - 4 || y >= H - 4) a = 0;
                px[i] = px[i].WithAlpha((byte)a);
            }
        }
        bmp.Pixels = px;
        return bmp;
    }

    static byte Clamp(int v) => (byte)Math.Clamp(v, 0, 255);

    static SKBitmap Load(string path)
    {
        using SKBitmap decoded = SKBitmap.Decode(path);
        var bmp = new SKBitmap(new SKImageInfo(decoded.Width, decoded.Height, SKColorType.Bgra8888, SKAlphaType.Premul));
        using var canvas = new SKCanvas(bmp);
        canvas.Clear(SKColors.Transparent);
        canvas.DrawBitmap(decoded, 0, 0);
        return bmp;
    }

    static void Save(SKBitmap bmp, string path)
    {
        using SKImage img = SKImage.FromBitmap(bmp);
        using SKData data = img.Encode(SKEncodedImageFormat.Png, 100);
        File.WriteAllBytes(path, data.ToArray());
    }
}
