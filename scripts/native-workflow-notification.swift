// Read-only macOS capability probe. Never asks for authorization or schedules alerts.
import AppKit
import UserNotifications
let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let center = UNUserNotificationCenter.current()
center.getNotificationSettings { settings in
    center.getPendingNotificationRequests { requests in
        let result: [String: Any] = [
            "bundleIdentifier": Bundle.main.bundleIdentifier ?? "missing",
            "authorizationStatus": settings.authorizationStatus.rawValue,
            "alertSetting": settings.alertSetting.rawValue,
            "pendingCount": requests.count,
            "requestedAuthorization": false,
            "scheduledNotification": false,
            "deliveryVerified": false
        ]
        let data = try! JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
        print(String(data: data, encoding: .utf8)!)
        fflush(stdout)
        exit(0)
    }
}
DispatchQueue.main.asyncAfter(deadline: .now() + 15) {
    fputs("notification settings timed out\n", stderr)
    exit(2)
}
app.run()
