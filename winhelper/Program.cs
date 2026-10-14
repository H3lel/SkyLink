// rootgo-bridge: calls into rootGo.dll exactly the way nthLink.SDK's RootGo class
// does on Windows, and moves every string through UTF-8 files so nothing depends
// on Wine's console code page.
//
//   rootgo-bridge <getconfig|feedback|report> <apikey-file> <data-file> <param-file> <out-file>
//
// Exit codes: 0 = call returned (out-file holds the raw result),
//             2 = Init failed (out-file holds the error), 64 = bad usage.
// Failure callbacks from the library are written to stderr, one JSON per line.
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

internal static class Program
{
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate void OnFailureDelegate(IntPtr error);

    [DllImport("rootGo.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr Init(IntPtr value);

    [DllImport("rootGo.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr GetConfig(IntPtr data, IntPtr jsonParam, OnFailureDelegate onFailure);

    [DllImport("rootGo.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr Report(IntPtr data, IntPtr jsonParam);

    [DllImport("rootGo.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr Feedback(IntPtr data, IntPtr jsonParam);

    [DllImport("rootGo.dll", CallingConvention = CallingConvention.Cdecl)]
    private static extern void FreeCString(IntPtr value);

    private static readonly UTF8Encoding Utf8 = new UTF8Encoding(false);

    private static int Main(string[] args)
    {
        if (args.Length != 5)
        {
            Console.Error.WriteLine("usage: rootgo-bridge <getconfig|feedback|report> <apikey-file> <data-file> <param-file> <out-file>");
            return 64;
        }
        string command = args[0];
        string apiKey = File.ReadAllText(args[1], Utf8).Trim();
        string data = File.Exists(args[2]) ? File.ReadAllText(args[2], Utf8).Trim() : string.Empty;
        string param = File.ReadAllText(args[3], Utf8);
        string outFile = args[4];

        string initError = CallAndFree(() => WithString(apiKey, Init));
        if (!string.IsNullOrEmpty(initError))
        {
            File.WriteAllText(outFile, initError, Utf8);
            return 2;
        }

        OnFailureDelegate onFailure = err => Console.Error.WriteLine(ReadUtf8(err));
        IntPtr dataPtr = ToUtf8(data);
        IntPtr paramPtr = ToUtf8(param);
        string result;
        try
        {
            result = command switch
            {
                "getconfig" => CallAndFree(() => GetConfig(dataPtr, paramPtr, onFailure)),
                "feedback" => CallAndFree(() => Feedback(dataPtr, paramPtr)),
                "report" => CallAndFree(() => Report(dataPtr, paramPtr)),
                _ => throw new ArgumentException("unknown command " + command),
            };
        }
        finally
        {
            Marshal.FreeHGlobal(dataPtr);
            Marshal.FreeHGlobal(paramPtr);
            GC.KeepAlive(onFailure);
        }
        File.WriteAllText(outFile, result, Utf8);
        return 0;
    }

    private static IntPtr WithString(string s, Func<IntPtr, IntPtr> f)
    {
        IntPtr p = ToUtf8(s);
        try { return f(p); }
        finally { Marshal.FreeHGlobal(p); }
    }

    private static string CallAndFree(Func<IntPtr> call)
    {
        IntPtr r = call();
        if (r == IntPtr.Zero) return string.Empty;
        string s = ReadUtf8(r);
        FreeCString(r);
        return s;
    }

    private static string ReadUtf8(IntPtr p) => p == IntPtr.Zero ? string.Empty : Marshal.PtrToStringUTF8(p) ?? string.Empty;

    private static IntPtr ToUtf8(string s)
    {
        byte[] b = Utf8.GetBytes(s);
        IntPtr p = Marshal.AllocHGlobal(b.Length + 1);
        Marshal.Copy(b, 0, p, b.Length);
        Marshal.WriteByte(p + b.Length, 0);
        return p;
    }
}
