import Cocoa
import ApplicationServices
import ScreenCaptureKit
import UniformTypeIdentifiers
import ImageIO
import Vision

func fail(_ msg: String) -> Never {
    fputs("error: \(msg)\n", stderr)
    exit(1)
}

func axGet(_ element: AXUIElement, _ attr: String) -> AXUIElement? {
    var value: AnyObject?
    guard AXUIElementCopyAttributeValue(element, attr as CFString, &value) == .success,
    let el = value else { return nil }
    return (el as! AXUIElement)
}

/*func axGetString(_ element: AXUIElement, _ attr: String) -> String? {
    var value: AnyObject?
    guard AXUIElementCopyAttributeValue(element, attr as CFString, &value) == .success,
    let str = value as? String else { return nil }
    return str
}*/

func axGetString(_ element: AXUIElement, _ attr: String) -> String? {
    var value: AnyObject?
    guard AXUIElementCopyAttributeValue(element, attr as CFString, &value) == .success else { return nil }
    if let str = value as? String {
        return str
    } else if let num = value as? NSNumber {
        return num.stringValue
    }
    return nil
}

func axPid(_ element: AXUIElement) -> pid_t {
    var pid: pid_t = 0
    AXUIElementGetPid(element, &pid)
    return pid
}

// ─── ElementPath ──────────────────────────────────────────────────────────────

struct ElementPath: Codable {
    let pid: Int32
    let steps: [Step]

    struct Step: Codable {
        let role: String
        let title: String
        let index: Int
    }
}

struct CaptureOutput: Codable {
    let path: ElementPath
    let debug: Debug

    struct Debug: Codable {
        let role: String
        let roleDescription: String
        let placeholder: String
        let stepCount: Int
    }
}

// ─── Модели данных для Dump-Screen (Новые) ──────────────────────────────────

struct FrameData: Codable {
    let x: Double
    let y: Double
    let w: Double
    let h: Double
}

struct AXNode: Codable {
    let role: String
    let title: String?
    let value: String?
    let description: String?
    let frame: FrameData?
    let children: [AXNode]?
}

struct OCRNode: Codable {
    let text: String
    let frame: FrameData
}

struct DumpOutput: Codable {
    let axTree: AXNode?
    let ocrText: [OCRNode]
}


func getAXFrame(_ element: AXUIElement) -> FrameData? {
    var posValue: AnyObject?
    var sizeValue: AnyObject?

    if AXUIElementCopyAttributeValue(element, kAXPositionAttribute as CFString, &posValue) == .success,
    AXUIElementCopyAttributeValue(element, kAXSizeAttribute as CFString, &sizeValue) == .success {

        if CFGetTypeID(posValue) == AXValueGetTypeID(), CFGetTypeID(sizeValue) == AXValueGetTypeID() {
            var point = CGPoint.zero
            var size = CGSize.zero

            let pValue = posValue as! AXValue
            let sValue = sizeValue as! AXValue

            if AXValueGetValue(pValue, .cgPoint, &point), AXValueGetValue(sValue, .cgSize, &size) {
                return FrameData(x: Double(point.x), y: Double(point.y), w: Double(size.width), h: Double(size.height))
            }
        }
    }
    return nil
}

func buildAXTree(element: AXUIElement, depth: Int = 0) -> AXNode? {
    // Ограничиваем глубину, чтобы не попасть в бесконечный цикл на огромных таблицах
    if depth > 15 { return nil }

    let role = axGetString(element, kAXRoleAttribute as String) ?? "Unknown"
    let title = axGetString(element, kAXTitleAttribute as String)
    let desc = axGetString(element, kAXDescriptionAttribute as String)
    let valStr = axGetString(element, kAXValueAttribute as String)

    let frame = getAXFrame(element)

    var childrenNodes: [AXNode] = []
    var childrenRef: AnyObject?

    if AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenRef) == .success,
    let children = childrenRef as? [AXUIElement] {
        for child in children {
            if let cNode = buildAXTree(element: child, depth: depth + 1) {
                childrenNodes.append(cNode)
            }
        }
    }

    // Оптимизация: не добавляем пустые роли без полезной инфы, если у них нет детей
    if childrenNodes.isEmpty && title == nil && valStr == nil && desc == nil && role == "Unknown" {
        return nil
    }

    return AXNode(
        role: role,
        title: title?.isEmpty == false ? title : nil,
        value: valStr?.isEmpty == false ? valStr : nil,
        description: desc?.isEmpty == false ? desc : nil,
        frame: frame,
        children: childrenNodes.isEmpty ? nil : childrenNodes
    )
}

// ─── Логика Apple Vision OCR ────────────────────────────────────────────────

func performOCR(imagePath: String) -> [OCRNode] {
    let url = URL(fileURLWithPath: imagePath)
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
    let cgImage = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
        return []
    }

    let imgWidth = Double(cgImage.width)
    let imgHeight = Double(cgImage.height)
    var ocrResults: [OCRNode] = []

    let request = VNRecognizeTextRequest { request, error in
        guard let observations = request.results as? [VNRecognizedTextObservation] else { return }
        for obs in observations {
            guard let candidate = obs.topCandidates(1).first else { continue }
            let text = candidate.string
            let bbox = obs.boundingBox

            // Переводим координаты Vision (origin: bottom-left) в координаты macOS (origin: top-left)
            let x = Double(bbox.origin.x) * imgWidth
            let y = (1.0 - Double(bbox.origin.y) - Double(bbox.size.height)) * imgHeight
            let w = Double(bbox.size.width) * imgWidth
            let h = Double(bbox.size.height) * imgHeight

            ocrResults.append(OCRNode(text: text, frame: FrameData(x: x, y: y, w: w, h: h)))
        }
    }

    request.recognitionLevel = .accurate
    // Включаем поддержку русского и английского языков
    request.recognitionLanguages = ["ru-RU", "en-US"]
    request.usesLanguageCorrection = true

    let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
    try? handler.perform([request])

    return ocrResults
}

// ─── Команды ──────────────────────────────────────────────────────────────────

func commandDumpScreen(pid: pid_t, imagePath: String) {
    let app = AXUIElementCreateApplication(pid)

    // 1. ФИКС: Ищем главное окно, чтобы не парсить системный MenuBar
    var targetElement = app
    var windowRef: AnyObject?

    if AXUIElementCopyAttributeValue(app, kAXMainWindowAttribute as CFString, &windowRef) == .success,
    let window = windowRef as! AXUIElement? {
        targetElement = window
    } else if AXUIElementCopyAttributeValue(app, kAXFocusedWindowAttribute as CFString, &windowRef) == .success,
    let window = windowRef as! AXUIElement? {
        targetElement = window
    }

    // Собираем семантическое дерево только для окна
    let axTree = buildAXTree(element: targetElement)

    // 2. Делаем OCR по скриншоту
    let ocrText = performOCR(imagePath: imagePath)

    let output = DumpOutput(axTree: axTree, ocrText: ocrText)

    let encoder = JSONEncoder()
    // Для Rust форматирование не нужно, сплошной JSON распарсится быстрее
    guard let data = try? encoder.encode(output),
    let json = String(data: data, encoding: .utf8) else {
        fail("Не удалось собрать Dump JSON")
    }

    print(json)
}


func buildPath(from root: AXUIElement, to target: AXUIElement, pid: pid_t) -> ElementPath? {
    var steps: [ElementPath.Step] = []

    func walk(_ element: AXUIElement, path: inout [ElementPath.Step]) -> Bool {
        if CFEqual(element, target) { return true }

        var childrenRef: AnyObject?
        guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &childrenRef) == .success,
        let children = childrenRef as? [AXUIElement] else { return false }

        var roleCounters: [String: Int] = [:]

        for child in children {
            let role  = axGetString(child, kAXRoleAttribute as String) ?? "AXUnknown"
            let idx   = roleCounters[role, default: 0]
            roleCounters[role] = idx + 1
            let title = axGetString(child, kAXTitleAttribute as String)
            ?? axGetString(child, kAXDescriptionAttribute as String) ?? ""

            path.append(ElementPath.Step(role: role, title: title, index: idx))

            if walk(child, path: &path) { return true }
            path.removeLast()
        }
        return false
    }

    guard walk(root, path: &steps) else { return nil }
    return ElementPath(pid: Int32(pid), steps: steps)
}

func resolvePath(_ path: ElementPath) -> AXUIElement? {
    var current: AXUIElement = AXUIElementCreateApplication(path.pid)

    for step in path.steps {
        var childrenRef: AnyObject?
        guard AXUIElementCopyAttributeValue(current, kAXChildrenAttribute as CFString, &childrenRef) == .success,
        let children = childrenRef as? [AXUIElement] else { return nil }

        var roleCounter = 0
        var found: AXUIElement?

        for child in children {
            let role = axGetString(child, kAXRoleAttribute as String) ?? "AXUnknown"
            guard role == step.role else { continue }
            if roleCounter == step.index { found = child; break }
            roleCounter += 1
        }

        guard let next = found else { return nil }
        current = next
    }
    return current
}

// ─── check-permissions ────────────────────────────────────────────────────────

func commandCheckPermissions() {
    struct Result: Codable {
        let accessibility: Bool
        let screenRecording: Bool
        let accessibilityMessage: String
        let screenRecordingMessage: String
    }

    let axOk = AXIsProcessTrusted()
    let screenOk = CGPreflightScreenCaptureAccess()

    let result = Result(
        accessibility: axOk,
        screenRecording: screenOk,
        accessibilityMessage: axOk
        ? "✅ Accessibility: OK"
        : "❌ Accessibility: нет прав\n   → System Settings → Privacy & Security → Accessibility",
        screenRecordingMessage: screenOk
        ? "✅ Screen Recording: OK"
        : "❌ Screen Recording: нет прав\n   → System Settings → Privacy & Security → Screen & System Audio Recording"
    )

    let encoder = JSONEncoder()
    encoder.outputFormatting = .prettyPrinted
    if let data = try? encoder.encode(result),
    let json = String(data: data, encoding: .utf8) {
        print(json)
    }

    fputs("\n\(result.accessibilityMessage)\n\(result.screenRecordingMessage)\n\n", stderr)
    exit(axOk && screenOk ? 0 : 1)
}

// ─── frontmost ────────────────────────────────────────────────────────────────

func commandFrontmost() {
    guard let app = NSWorkspace.shared.frontmostApplication else {
        fail("Нет активного приложения")
    }
    print("\(app.processIdentifier)|\(app.localizedName ?? "Unknown")")
}

// ─── capture ──────────────────────────────────────────────────────────────────

func commandCapture(pid: pid_t) {
    let app     = AXUIElementCreateApplication(pid)
    let sysWide = AXUIElementCreateSystemWide()

    // 1. Поиск целевого элемента (с каскадным фолбеком)
    var targetElement = axGet(sysWide, kAXFocusedUIElementAttribute as String)
    if targetElement == nil {
        targetElement = axGet(app, kAXFocusedUIElementAttribute as String)
    }
    if targetElement == nil {
        targetElement = axGet(app, kAXMainWindowAttribute as String)
    }

    guard let focused = targetElement else {
        fail("Нет сфокусированного элемента или открытого окна")
        return
    }

    // Проверка на принадлежность процессу
    var elementPid: pid_t = 0
    AXUIElementGetPid(focused, &elementPid)
    guard elementPid == pid else {
        fail("Сфокусированный элемент принадлежит другому процессу")
        return
    }

    // 2. Пытаемся построить путь через твой buildPath
    // Я предполагаю, что buildPath возвращает ElementPath?
    var finalPath: ElementPath? = buildPath(from: app, to: focused, pid: pid)

    // 3. Фолбек для Telegram/Slack/Electron
    // Если иерархия сломана и путь не строится, создаем путь из одной точки
    if finalPath == nil {
        let role = axGetString(focused, kAXRoleAttribute as String) ?? "AXUnknown"
        let title = axGetString(focused, kAXTitleAttribute as String) ?? ""

        // Создаем шаг (используй точное название своей структуры шага,
        // скорее всего это ElementPath.Step или PathStep)
        let manualStep = ElementPath.Step(role: role, title: title, index: 0)
        finalPath = ElementPath(pid: pid,steps: [manualStep])
    }

    guard let path = finalPath else {
        fail("Не удалось построить путь до элемента")
        return
    }

    // 4. Сбор отладочной информации
    let role        = axGetString(focused, kAXRoleAttribute as String)            ?? "unknown"
    let roleDesc    = axGetString(focused, kAXRoleDescriptionAttribute as String) ?? ""
    let placeholder = axGetString(focused, kAXPlaceholderValueAttribute as String) ?? ""

    let out = CaptureOutput(
        path: path,
        debug: .init(
            role: role,
            roleDescription: roleDesc,
            placeholder: placeholder,
            stepCount: path.steps.count
        )
    )

    // 5. Вывод JSON
    let encoder = JSONEncoder()
    encoder.outputFormatting = .prettyPrinted
    guard let data = try? encoder.encode(out),
    let json = String(data: data, encoding: .utf8) else {
        fail("JSON ошибка")
        return
    }

    print(json)
}

/*
func commandCapture(pid: pid_t) {
    let app     = AXUIElementCreateApplication(pid)
    let sysWide = AXUIElementCreateSystemWide()

    // 1. Попытка: системный запрос фокуса (как было у тебя)
    var targetElement = axGet(sysWide, kAXFocusedUIElementAttribute as String)

    // 2. Фолбек А: Запрашиваем фокус напрямую у процесса.
    // Это часто спасает в Notes, Safari и многих других приложениях.
    if targetElement == nil {
        targetElement = axGet(app, kAXFocusedUIElementAttribute as String)
    }

    // 3. Фолбек Б: Если текстовое поле или каретка вообще не отдают фокус,
    // цепляемся за главное окно приложения, чтобы не ронять пайплайн
    // и вернуть Rust'у хоть какой-то валидный path.
    if targetElement == nil {
        targetElement = axGet(app, kAXMainWindowAttribute as String)
    }

    // Если все 3 попытки провалились
    guard let focused = targetElement else {
        fail("Нет сфокусированного элемента или открытого окна")
        return
    }

    guard axPid(focused) == pid else {
        fail("Сфокусированный элемент принадлежит другому процессу")
        return
    }

    guard let path = buildPath(from: app, to: focused, pid: pid) else {
        fail("Не удалось построить путь до элемента")
        return
    }

    let role        = axGetString(focused, kAXRoleAttribute as String)            ?? "unknown"
    let roleDesc    = axGetString(focused, kAXRoleDescriptionAttribute as String) ?? ""
    let placeholder = axGetString(focused, kAXPlaceholderValueAttribute as String) ?? ""

    let out = CaptureOutput(path: path, debug: .init(
        role: role, roleDescription: roleDesc,
        placeholder: placeholder, stepCount: path.steps.count
    ))

    let encoder = JSONEncoder()
    encoder.outputFormatting = .prettyPrinted
    guard let data = try? encoder.encode(out),
    let json = String(data: data, encoding: .utf8) else {
        fail("JSON ошибка")
        return
    }

    print(json)
}*/

/*func commandCapture(pid: pid_t) {
    let app     = AXUIElementCreateApplication(pid)
    let sysWide = AXUIElementCreateSystemWide()

    guard let focused = axGet(sysWide, kAXFocusedUIElementAttribute as String) else {
        fail("Нет сфокусированного элемента")
    }
    guard axPid(focused) == pid else {
        fail("Сфокусированный элемент принадлежит другому процессу")
    }
    guard let path = buildPath(from: app, to: focused, pid: pid) else {
        fail("Не удалось построить путь до элемента")
    }

    let role        = axGetString(focused, kAXRoleAttribute as String)            ?? "unknown"
    let roleDesc    = axGetString(focused, kAXRoleDescriptionAttribute as String) ?? ""
    let placeholder = axGetString(focused, kAXPlaceholderValueAttribute as String) ?? ""

    let out = CaptureOutput(path: path, debug: .init(
        role: role, roleDescription: roleDesc,
        placeholder: placeholder, stepCount: path.steps.count
    ))

    let encoder = JSONEncoder()
    encoder.outputFormatting = .prettyPrinted
    guard let data = try? encoder.encode(out),
    let json = String(data: data, encoding: .utf8) else { fail("JSON ошибка") }

    print(json)
}*/

// ─── inject ───────────────────────────────────────────────────────────────────

func commandInject(pathJSON: String, text: String) {
    guard let data    = pathJSON.data(using: .utf8),
    // ДЕКОДИРУЕМ CaptureOutput ВМЕСТО ElementPath
    let decoded = try? JSONDecoder().decode(CaptureOutput.self, from: data) else {
        fail("Невалидный JSON пути")
    }

    // ДОСТАЕМ PID И ПУТЬ ИЗ ОБЕРТКИ
    let pid = decoded.path.pid
    let elementPath = decoded.path

    if let app = NSRunningApplication(processIdentifier: pid) {
        app.activate(options: [])
        Thread.sleep(forTimeInterval: 0.08)
    }

    if let element = resolvePath(elementPath) {
        AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, true as CFTypeRef)
        Thread.sleep(forTimeInterval: 0.05)
    } else {
        fputs("warning: элемент не найден, вставляю в активный фокус\n", stderr)
    }

    injectViaClipboard(pid: pid, text: text)
}

func injectViaClipboard(pid: pid_t, text: String) {
    let pb       = NSPasteboard.general
    let original = pb.string(forType: .string)
    pb.clearContents()
    pb.setString(text, forType: .string)
    Thread.sleep(forTimeInterval: 0.05)

    let src     = CGEventSource(stateID: .hidSystemState)
    let keyDown = CGEvent(keyboardEventSource: src, virtualKey: 0x09, keyDown: true)!
    keyDown.flags = .maskCommand
    keyDown.postToPid(pid)
    Thread.sleep(forTimeInterval: 0.02)

    let keyUp = CGEvent(keyboardEventSource: src, virtualKey: 0x09, keyDown: false)!
    keyUp.flags = .maskCommand
    keyUp.postToPid(pid)
    Thread.sleep(forTimeInterval: 0.15)

    pb.clearContents()
    if let orig = original { pb.setString(orig, forType: .string) }
}

// ─── screenshot (НОВОЕ) ───────────────────────────────────────────────────────

@available(macOS 14.0, *)
func commandScreenshot(outputPath: String) async {
    do {
        // Получаем доступные экраны (исключая системные элементы вроде обоев, если нужно)
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)

        // Берем основной экран
        guard let display = content.displays.first else {
            fail("Не удалось найти дисплей для скриншота")
        }

        let filter = SCContentFilter(display: display, excludingWindows: [])
        let config = SCStreamConfiguration()
        config.width = display.width
        config.height = display.height

        // Делаем снимок через ScreenCaptureKit
        let cgImage = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)

        // Сохраняем как PNG
        let url = URL(fileURLWithPath: outputPath)
        guard let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil) else {
            fail("Не удалось создать файл по пути \(outputPath)")
        }

        CGImageDestinationAddImage(destination, cgImage, nil)
        guard CGImageDestinationFinalize(destination) else {
            fail("Не удалось финализировать изображение")
        }

        print("{\"status\":\"success\", \"path\":\"\(outputPath)\"}")

    } catch {
        fail("Ошибка при создании скриншота: \(error.localizedDescription)")
    }
}


// ─── Entry point ──────────────────────────────────────────────────────────────

let args = CommandLine.arguments
guard args.count >= 2 else {
    fail("Usage:\n  ax-helper check-permissions\n  ax-helper frontmost\n  ax-helper capture <pid>\n  ax-helper inject <pid> <path-json> <text>\n  ax-helper screenshot <output.png>\n  ax-helper dump-screen <pid> <imagePath>")
}

switch args[1] {
case "check-permissions":
    commandCheckPermissions()

case "frontmost":
    commandFrontmost()

case "capture":
    guard args.count >= 3, let pid = pid_t(args[2]) else { fail("Невалидный PID") }
    commandCapture(pid: pid)

case "inject":
    guard args.count >= 5 else { fail("inject требует: <pid> <path-json-file> <text>") }
    guard let pathJSON = try? String(contentsOfFile: args[3], encoding: .utf8) else {
        fail("Не могу прочитать файл: \(args[3])")
    }
    commandInject(pathJSON: pathJSON, text: args[4])

case "screenshot":
    guard args.count >= 3 else { fail("screenshot требует путь: ax-helper screenshot <output.png>") }
    if #available(macOS 14.0, *) {
        // Запускаем асинхронную функцию
        await commandScreenshot(outputPath: args[2])
    } else {
        fail("Скриншоты через ScreenCaptureKit требуют macOS 14.0+")
    }

case "dump-screen":
    guard args.count >= 4, let pid = pid_t(args[2]) else { fail("Usage: dump-screen <pid> <imagePath>") }
    commandDumpScreen(pid: pid, imagePath: args[3])

default:
    fail("Неизвестная команда: \(args[1])")
}