using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;

// NOTE: keep this file pure ASCII. PowerShell 5.1 Get-Content reads it as the
// system ANSI code page (GBK on this machine), so any UTF-8 CJK comment gets
// re-decoded into arbitrary bytes and can swallow a brace/quote, which makes
// Add-Type report bogus "invalid token in class member declaration" errors.
public class Grab
{
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int L, T, R, B; }

    public delegate bool EnumProc(IntPtr h, IntPtr l);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumProc cb, IntPtr l);

    [DllImport("user32.dll")]
    public static extern bool EnumChildWindows(IntPtr p, EnumProc cb, IntPtr l);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr h, out RECT r);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr h);

    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);

    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);

    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int x, int y);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr h);

    // delta > 0 wheels away from the user (content scrolls up).
    public static string Wheel(string cls, int notches)
    {
        IntPtr h = Find(cls);
        if (h == IntPtr.Zero) return "NOT FOUND: " + cls;
        int d = 120 * notches;
        IntPtr w = new IntPtr((d << 16) & 0xFFFFFFFF);
        for (int i = 0; i < System.Math.Abs(notches); i++)
            PostMessageW(h, 0x020A, w, IntPtr.Zero);
        return "posted " + System.Math.Abs(notches) + " notches to " + cls;
    }

    public static string MoveMouse(int x, int y)
    {
        return SetCursorPos(x, y) ? "mouse " + x + "," + y : "SetCursorPos failed";
    }

    public static string Foreground(string cls)
    {
        IntPtr h = Find(cls);
        if (h == IntPtr.Zero) return "NOT FOUND: " + cls;
        SetForegroundWindow(h);
        return "foreground set";
    }

    // Post WM_COMMAND (0x0111) to a window, e.g. to drive the tray menu.
    public static string PostCmd(string cls, int cmdId)
    {
        IntPtr h = Find(cls);
        if (h == IntPtr.Zero) return "NOT FOUND: " + cls;
        return PostMessageW(h, 0x0111, new IntPtr(cmdId), IntPtr.Zero)
            ? "posted WM_COMMAND " + cmdId + " to " + cls
            : "PostMessageW failed";
    }

    // Generic PostMessage wrapper, e.g. WM_MOUSEMOVE to arm the hover timer.
    public static string Post(string cls, int msg, int wparam)
    {
        IntPtr h = Find(cls);
        if (h == IntPtr.Zero) return "NOT FOUND: " + cls;
        return PostMessageW(h, (uint)msg, new IntPtr(wparam), IntPtr.Zero)
            ? "posted " + msg + " to " + cls
            : "PostMessageW failed";
    }

    // Post a mouse click at CLIENT coordinates (x packed low, y packed high).
    public static string ClickAt(string cls, int x, int y)
    {
        IntPtr h = Find(cls);
        if (h == IntPtr.Zero) return "NOT FOUND: " + cls;
        IntPtr lp = new IntPtr((y << 16) | (x & 0xFFFF));
        PostMessageW(h, 0x0201, new IntPtr(1), lp); // WM_LBUTTONDOWN
        PostMessageW(h, 0x0202, IntPtr.Zero, lp);   // WM_LBUTTONUP
        return "clicked " + cls + " at " + x + "," + y;
    }

    // Park the cursor over the middle of a window (hit-test forwarding path).
    public static string Hover(string cls)
    {
        IntPtr h = Find(cls);
        if (h == IntPtr.Zero) return "NOT FOUND: " + cls;
        RECT r;
        GetWindowRect(h, out r);
        int x = (r.L + r.R) / 2, y = (r.T + r.B) / 2;
        return SetCursorPos(x, y)
            ? "hover " + cls + " at " + x + "," + y
            : "SetCursorPos failed";
    }

    static string ClsOf(IntPtr h)
    {
        StringBuilder sb = new StringBuilder(256);
        GetClassNameW(h, sb, 256);
        return sb.ToString();
    }

    static IntPtr Scan(bool children, IntPtr root, string cls)
    {
        IntPtr hit = IntPtr.Zero;
        EnumProc cb = delegate(IntPtr h, IntPtr l)
        {
            if (ClsOf(h) == cls) { hit = h; return false; }
            return true;
        };
        if (children) EnumChildWindows(root, cb, IntPtr.Zero);
        else EnumWindows(cb, IntPtr.Zero);
        return hit;
    }

    // Top-level windows first, then children of every Shell_TrayWnd.
    public static IntPtr Find(string cls)
    {
        IntPtr h = Scan(false, IntPtr.Zero, cls);
        if (h != IntPtr.Zero) return h;
        h = Scan(false, IntPtr.Zero, "Shell_TrayWnd");
        if (h == IntPtr.Zero) return IntPtr.Zero;
        return Scan(true, h, cls);
    }

    static void Scale(Bitmap src, string path, int zoom)
    {
        Bitmap big = new Bitmap(src.Width * zoom, src.Height * zoom);
        Graphics g = Graphics.FromImage(big);
        g.InterpolationMode = System.Drawing.Drawing2D.InterpolationMode.NearestNeighbor;
        g.PixelOffsetMode = System.Drawing.Drawing2D.PixelOffsetMode.Half;
        g.DrawImage(src, 0, 0, src.Width * zoom, src.Height * zoom);
        g.Dispose();
        big.Save(path, ImageFormat.Png);
        big.Dispose();
    }

    // PrintWindow renders the window offscreen, so it works even when a
    // fullscreen game is covering the taskbar on the primary monitor.
    public static string Shot(string cls, string outBase, int zoom)
    {
        IntPtr h = Find(cls);
        if (h == IntPtr.Zero) return "NOT FOUND: " + cls;

        RECT r;
        GetWindowRect(h, out r);
        int w = r.R - r.L;
        int ht = r.B - r.T;
        if (w <= 0 || ht <= 0) return "BAD RECT";

        Bitmap bmp = new Bitmap(w, ht);
        Graphics g = Graphics.FromImage(bmp);
        IntPtr hdc = g.GetHdc();
        bool ok = PrintWindow(h, hdc, 2);
        g.ReleaseHdc(hdc);
        g.Dispose();
        if (!ok) { bmp.Dispose(); return "PrintWindow failed"; }

        bmp.Save(outBase + "_1x.png", ImageFormat.Png);
        if (zoom > 1) Scale(bmp, outBase + "_" + zoom + "x.png", zoom);
        bmp.Dispose();
        return "OK " + cls + " " + w + "x" + ht + " -> " + outBase + "_1x.png";
    }
}
