using System;
using System.IO;
using System.Threading;
using Windows.Foundation;
using Windows.Media.Control;
using Windows.Storage.Streams;

public static class AlbumArt
{
    // Block on a WinRT async op without needing System.Runtime.WindowsRuntime extensions.
    static T Await<T>(IAsyncOperation<T> op)
    {
        var done = new ManualResetEventSlim(false);
        op.Completed = (o, status) => done.Set();
        done.Wait();
        return op.GetResults();
    }

    public static int Main(string[] args)
    {
        string outPath = args.Length > 0 ? args[0] : "albumart.png";
        try
        {
            var mgr = Await(GlobalSystemMediaTransportControlsSessionManager.RequestAsync());

            GlobalSystemMediaTransportControlsSession target = null;
            foreach (var s in mgr.GetSessions())
            {
                if (s.SourceAppUserModelId != null &&
                    s.SourceAppUserModelId.IndexOf("Spotify", StringComparison.OrdinalIgnoreCase) >= 0)
                    target = s;
            }
            if (target == null) target = mgr.GetCurrentSession();
            if (target == null) { Console.WriteLine("NO_SESSION"); return 1; }

            var props = Await(target.TryGetMediaPropertiesAsync());
            Console.WriteLine("SOURCE: " + target.SourceAppUserModelId);
            Console.WriteLine("TITLE: " + props.Title);
            Console.WriteLine("ARTIST: " + props.Artist);
            Console.WriteLine("ALBUM: " + props.AlbumTitle);

            if (props.Thumbnail == null) { Console.WriteLine("NO_THUMBNAIL"); return 1; }

            var ras = Await(props.Thumbnail.OpenReadAsync());
            uint size = (uint)ras.Size;
            var reader = new DataReader(ras.GetInputStreamAt(0));
            Await((IAsyncOperation<uint>)reader.LoadAsync(size));
            byte[] bytes = new byte[size];
            reader.ReadBytes(bytes);
            File.WriteAllBytes(outPath, bytes);

            Console.WriteLine("SAVED: " + outPath + " bytes=" + bytes.Length);
            return 0;
        }
        catch (Exception ex)
        {
            Console.WriteLine("ERROR: " + ex);
            return 1;
        }
    }
}
