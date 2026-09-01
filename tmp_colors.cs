using System;
using System.Drawing;
using System.Drawing.Drawing2D;

public static class DominantColors
{
    public static int Main(string[] args)
    {
        string path = args[0];
        int k = args.Length > 1 ? int.Parse(args[1]) : 3;

        // Downscale so every pixel counts once and the pass is cheap.
        int n = 128;
        var pixels = new double[n * n][];
        using (var src = new Bitmap(path))
        using (var small = new Bitmap(n, n))
        {
            using (var g = Graphics.FromImage(small))
            {
                g.InterpolationMode = InterpolationMode.HighQualityBilinear;
                g.DrawImage(src, 0, 0, n, n);
            }
            for (int y = 0; y < n; y++)
                for (int x = 0; x < n; x++)
                {
                    var c = small.GetPixel(x, y);
                    pixels[y * n + x] = new double[] { c.R, c.G, c.B };
                }
        }

        // k-means++ init with a fixed seed => reproducible output.
        var rng = new Random(1234);
        var cent = new double[k][];
        cent[0] = (double[])pixels[rng.Next(pixels.Length)].Clone();
        for (int i = 1; i < k; i++)
        {
            var d2 = new double[pixels.Length];
            double sum = 0;
            for (int p = 0; p < pixels.Length; p++)
            {
                double best = double.MaxValue;
                for (int c = 0; c < i; c++)
                {
                    double d = Dist(pixels[p], cent[c]);
                    if (d < best) best = d;
                }
                d2[p] = best;
                sum += best;
            }
            double r = rng.NextDouble() * sum, acc = 0;
            int pick = pixels.Length - 1;
            for (int p = 0; p < pixels.Length; p++)
            {
                acc += d2[p];
                if (acc >= r) { pick = p; break; }
            }
            cent[i] = (double[])pixels[pick].Clone();
        }

        var assign = new int[pixels.Length];
        for (int iter = 0; iter < 60; iter++)
        {
            bool changed = false;
            for (int p = 0; p < pixels.Length; p++)
            {
                int best = 0; double bd = double.MaxValue;
                for (int c = 0; c < k; c++)
                {
                    double d = Dist(pixels[p], cent[c]);
                    if (d < bd) { bd = d; best = c; }
                }
                if (assign[p] != best) { assign[p] = best; changed = true; }
            }
            var sums = new double[k][];
            var cnt = new int[k];
            for (int c = 0; c < k; c++) sums[c] = new double[3];
            for (int p = 0; p < pixels.Length; p++)
            {
                int c = assign[p];
                for (int j = 0; j < 3; j++) sums[c][j] += pixels[p][j];
                cnt[c]++;
            }
            for (int c = 0; c < k; c++)
                if (cnt[c] > 0)
                    for (int j = 0; j < 3; j++) cent[c][j] = sums[c][j] / cnt[c];
            if (!changed) break;
        }

        var counts = new int[k];
        for (int p = 0; p < pixels.Length; p++) counts[assign[p]]++;

        var order = new int[k];
        for (int i = 0; i < k; i++) order[i] = i;
        Array.Sort((int[])counts.Clone(), order);
        Array.Reverse(order);

        foreach (int c in order)
        {
            int R = (int)Math.Round(cent[c][0]);
            int G = (int)Math.Round(cent[c][1]);
            int B = (int)Math.Round(cent[c][2]);
            double share = 100.0 * counts[c] / pixels.Length;
            var col = Color.FromArgb(R, G, B);
            Console.WriteLine(string.Format(
                "#{0:X2}{1:X2}{2:X2}  rgb({3,3},{4,3},{5,3})  hsl({6,3},{7,3}%,{8,3}%)  {9,5:F1}%",
                R, G, B, R, G, B,
                (int)Math.Round(col.GetHue()),
                (int)Math.Round(col.GetSaturation() * 100),
                (int)Math.Round(col.GetBrightness() * 100),
                share));
        }
        return 0;
    }

    static double Dist(double[] a, double[] b)
    {
        double s = 0;
        for (int i = 0; i < 3; i++) { double d = a[i] - b[i]; s += d * d; }
        return s;
    }
}
