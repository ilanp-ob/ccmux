import Cocoa
import UserNotifications

// Usage: ccmux-notify <title> <subtitle> <body>
// Requests permission on first run, then delivers a banner notification.

let args = CommandLine.arguments
guard args.count >= 4 else {
    fputs("usage: ccmux-notify <title> <subtitle> <body>\n", stderr)
    exit(1)
}

let title    = args[1]
let subtitle = args[2]
let body     = args[3]

class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    func applicationDidFinishLaunching(_ note: Notification) {
        let center = UNUserNotificationCenter.current()
        center.delegate = self

        center.requestAuthorization(options: [.alert, .sound, .badge]) { granted, error in
            guard granted else {
                fputs("ccmux-notify: permission denied\n", stderr)
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { NSApp.terminate(nil) }
                return
            }

            let content = UNMutableNotificationContent()
            content.title    = title
            content.subtitle = subtitle
            content.body     = body
            content.sound    = .default

            let request = UNNotificationRequest(
                identifier: UUID().uuidString,
                content: content,
                trigger: nil          // deliver immediately
            )
            center.add(request) { err in
                if let err { fputs("ccmux-notify: \(err)\n", stderr) }
                // Give the system a moment to display the banner before we exit.
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { NSApp.terminate(nil) }
            }
        }
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler handler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        handler([.banner, .sound])
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)   // no Dock icon, no menu bar
app.run()
