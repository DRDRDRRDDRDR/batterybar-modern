using System;
using System.Text;
using System.Runtime.InteropServices;

public class WEnum {
  [StructLayout(LayoutKind.Sequential)]
  public struct RECT { public int L, T, R, B; }

  public delegate bool EnumProc(IntPtr h, IntPtr l);

  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc cb, IntPtr l);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr h);

  public static string Dump(string filter) {
    var sb = new StringBuilder();
    Walk(IntPtr.Zero, filter, sb, "top");
    // 找任务栏
    IntPtr tray = IntPtr.Zero;
    EnumWindows((h, l) => {
      var c = new StringBuilder(256); GetClassNameW(h, c, 256);
      if (c.ToString() == "Shell_TrayWnd") { tray = h; return false; }
      return true;
    }, IntPtr.Zero);
    if (tray != IntPtr.Zero) {
      RECT tr; GetWindowRect(tray, out tr);
      sb.AppendLine(string.Format("Shell_TrayWnd  hwnd=0x{0:X}  ({1},{2}) {3}x{4}", tray.ToInt64(), tr.L, tr.T, tr.R - tr.L, tr.B - tr.T));
      Walk(tray, filter, sb, "taskbar");
    }
    return sb.ToString();
  }

  static void Walk(IntPtr parent, string filter, StringBuilder sb, string tag) {
    EnumProc cb = null;
    cb = delegate(IntPtr h, IntPtr l) {
      var c = new StringBuilder(256);
      GetClassNameW(h, c, 256);
      string cls = c.ToString();
      if (cls.StartsWith(filter, StringComparison.OrdinalIgnoreCase)) {
        RECT r; GetWindowRect(h, out r);
        uint pid; GetWindowThreadProcessId(h, out pid);
        IntPtr par = GetParent(h);
        sb.AppendLine(string.Format(
          "[{0}] cls={1} hwnd=0x{2:X} pid={3} parent=0x{4:X} rect=({5},{6})-({7},{8}) {9}x{10} vis={11}",
          tag, cls, h.ToInt64(), pid, par.ToInt64(), r.L, r.T, r.R, r.B, r.R - r.L, r.B - r.T, IsWindowVisible(h)));
      }
      return true;
    };
    if (parent == IntPtr.Zero) EnumWindows(cb, IntPtr.Zero);
    else EnumChildWindows(parent, cb, IntPtr.Zero);
  }
}
