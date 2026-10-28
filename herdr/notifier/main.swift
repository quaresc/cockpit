// Cockpit's macOS notifications: the cockpit logo, a click that opens a URL or runs `cockpit focus` with the action, and
// buttons that do the same with their own action.
//   cockpit-notify post <title> <body> [action] [<label>=<action> ...]
//   cockpit-notify status    prints whether macOS lets Cockpit notify
import Cocoa
import UserNotifications

final class Notifier: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    let center = UNUserNotificationCenter.current()

    func applicationDidFinishLaunching(_ notification: Notification) {
        center.delegate = self
        let args = CommandLine.arguments
        if args.contains("status") {
            center.getNotificationSettings { settings in
                let names = ["not asked yet", "denied", "allowed", "provisional", "ephemeral"]
                let status = settings.authorizationStatus.rawValue
                print(status < names.count ? names[status] : "unknown")
                self.quit(after: 0)
            }
        } else if let i = args.firstIndex(of: "post"), args.count > i + 2 {
            let buttons = args.dropFirst(i + 4).compactMap { pair -> (String, String)? in
                guard let eq = pair.firstIndex(of: "=") else { return nil }
                return (String(pair[..<eq]), String(pair[pair.index(after: eq)...]))
            }
            post(title: args[i + 1], body: args[i + 2], action: args.count > i + 3 ? args[i + 3] : "", buttons: buttons)
        } else {
            // Launched by a click: the response arrives through the delegate, then the app leaves.
            quit(after: 10)
        }
    }

    func post(title: String, body: String, action: String, buttons: [(String, String)]) {
        center.requestAuthorization(options: [.alert, .sound]) { granted, error in
            if !granted { FileHandle.standardError.write("cockpit-notify: not allowed (\(error?.localizedDescription ?? "denied in System Settings"))\n".data(using: .utf8)!) }
            let content = UNMutableNotificationContent()
            content.title = title
            content.body = body
            content.sound = .default
            var info: [String: String] = ["action": action]
            for (n, button) in buttons.enumerated() { info["button\(n)"] = button.1 }
            content.userInfo = info
            // The buttons' labels are a category of their own, registered beside the ones already known.
            let category = "buttons:" + buttons.map { $0.0 }.joined(separator: "|")
            if !buttons.isEmpty { content.categoryIdentifier = category }
            self.center.getNotificationCategories { known in
                var categories = known.filter { $0.identifier != category }
                if !buttons.isEmpty {
                    let actions = buttons.enumerated().map { UNNotificationAction(identifier: "button\($0.offset)", title: $0.element.0, options: [.foreground]) }
                    categories.insert(UNNotificationCategory(identifier: category, actions: actions, intentIdentifiers: []))
                }
                self.center.setNotificationCategories(categories)
                // macOS registers categories asynchronously: a notification posted before its category lands has no buttons.
                self.whenRegistered(category, tries: buttons.isEmpty ? 0 : 20) {
                    let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
                    self.center.add(request) { error in
                        if let error { FileHandle.standardError.write("cockpit-notify: \(error.localizedDescription)\n".data(using: .utf8)!) }
                        self.quit(after: 0.3)
                    }
                }
            }
        }
    }

    func whenRegistered(_ category: String, tries: Int, then post: @escaping () -> Void) {
        guard tries > 0 else { return post() }
        center.getNotificationCategories { known in
            if known.contains(where: { $0.identifier == category }) { return post() }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { self.whenRegistered(category, tries: tries - 1, then: post) }
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                withCompletionHandler done: @escaping () -> Void) {
        let info = response.notification.request.content.userInfo
        let pressed = response.actionIdentifier.hasPrefix("button") ? info[response.actionIdentifier] as? String : nil
        let action = pressed ?? info["action"] as? String ?? ""
        if action.hasPrefix("http://") || action.hasPrefix("https://"), let url = URL(string: action) {
            NSWorkspace.shared.open(url)
        } else {
            let task = Process()
            task.executableURL = URL(fileURLWithPath: "/bin/sh")
            task.arguments = [NSHomeDirectory() + "/.config/cockpit/herdr/cockpit.sh", "focus", action]
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
