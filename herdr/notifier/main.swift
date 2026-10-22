// Cockpit's macOS notifications: the cockpit logo, and a click that opens a URL or runs focus.sh with the action.
//   cockpit-notify post <title> <body> [action]
import Cocoa
import UserNotifications

final class Notifier: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    let center = UNUserNotificationCenter.current()

    func applicationDidFinishLaunching(_ notification: Notification) {
        center.delegate = self
        let args = CommandLine.arguments
        if let i = args.firstIndex(of: "post"), args.count > i + 2 {
            post(title: args[i + 1], body: args[i + 2], action: args.count > i + 3 ? args[i + 3] : "")
        } else {
            // Launched by a click: the response arrives through the delegate, then the app leaves.
            quit(after: 10)
        }
    }

    func post(title: String, body: String, action: String) {
        center.requestAuthorization(options: [.alert, .sound]) { _, _ in
            let content = UNMutableNotificationContent()
            content.title = title
            content.body = body
            content.sound = .default
            content.userInfo = ["action": action]
            let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
            self.center.add(request) { _ in self.quit(after: 0.3) }
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                withCompletionHandler done: @escaping () -> Void) {
        let action = response.notification.request.content.userInfo["action"] as? String ?? ""
        if action.hasPrefix("http://") || action.hasPrefix("https://"), let url = URL(string: action) {
            NSWorkspace.shared.open(url)
        } else {
            let task = Process()
            task.executableURL = URL(fileURLWithPath: "/bin/sh")
            task.arguments = [NSHomeDirectory() + "/.config/cockpit/herdr/focus.sh", action]
            try? task.run()
            task.waitUntilExit()
        }
        done()
        quit(after: 0.5)
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler show: @escaping (UNNotificationPresentationOptions) -> Void) {
        show([.banner, .sound, .list])
    }

    func quit(after seconds: Double) {
        DispatchQueue.main.asyncAfter(deadline: .now() + seconds) { NSApp.terminate(nil) }
    }
}

let app = NSApplication.shared
let notifier = Notifier()
app.delegate = notifier
app.setActivationPolicy(.prohibited)
app.run()
