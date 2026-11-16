// Secpar desktop app. One window that connects to computers and controls hosting.
//
// Development options:
//   --sample                 fill the pages with fictional layout data
//   --sample-open            with --sample, show a dropdown open (Settings) or a
//                            friend's permissions open (Friends), for captures
//   --tab <0|1|2>            open on Computers, Friends or Settings
//   --section <0-5>          open Settings on this section
//   --computer-name <name>   enrol this computer under another name than its own
//   --size <W>x<H>           window size in logical pixels
//   --screenshot <file.png>  save the window to a file and quit
//   --screenshot-delay <ms>  wait this long before saving (default 1200)
//   --channel <name>         the background part's channel
//   --test-account <origin> <address>   use a test account service on this computer
//   --memory-store           keep the session out of the system's password store
//   --test-sign-in <file>    sign in as the first account of a local test accounts file
//   --test-connect           three seconds later, connect to the first computer that is on
#include "backend.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QFont>
#include <QApplication>
#include <QCoreApplication>
#include <QJsonArray>
#include <QLocalSocket>
#include <QProcess>
#include <QSettings>
#include <QThread>
#include <cstring>
#include <QJsonDocument>
#include <QStandardPaths>
#include <QImage>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QTimer>

#ifdef Q_OS_WIN
#include <windows.h>
#include <string>
// Starts the background part of this app — secpar-service.exe with the engine
// that was installed beside it — without a console window, then returns. The
// installer and the login entry both run this, so no part of the app is
// separate from it.
static int startHost()
{
    const QDir app_dir(QCoreApplication::applicationDirPath());
    const QString service = app_dir.filePath("secpar-service.exe");
    const QString engine = app_dir.filePath("engine/sunshine.exe");
    if (!QFile::exists(service) || !QFile::exists(engine))
        return 2;
    // The background part keeps this computer's identity and the engine's own
    // state here; it is never inside the program folder, so an update and an
    // uninstall leave it alone.
    const QString data = qEnvironmentVariable("LOCALAPPDATA") + "/Secpar4";
    const QString command = "\"" + QDir::toNativeSeparators(service) + "\""
            + " --state \"" + QDir::toNativeSeparators(data) + "\""
            + " --channel secpar-service"
            + " --engine \"" + QDir::toNativeSeparators(engine) + "\""
            + " --tunnel-port 46900";
    std::wstring line = command.toStdWString();
    std::wstring directory = QDir::toNativeSeparators(app_dir.absolutePath()).toStdWString();
    STARTUPINFOW startup{};
    startup.cb = sizeof(startup);
    PROCESS_INFORMATION process{};
    // No visible console, but a real one: the hosting engine writes to its
    // standard handles, and with DETACHED_PROCESS it has none and dies as soon as
    // it starts. CREATE_NO_WINDOW keeps a console that is never shown.
    // CREATE_BREAKAWAY_FROM_JOB lets it outlive the installer that started it; a
    // job that does not allow breakaway makes CreateProcess fail outright, so the
    // same start is attempted once without it.
    const DWORD flags = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB;
    BOOL started = CreateProcessW(nullptr, line.data(), nullptr, nullptr, FALSE, flags, nullptr,
                                  directory.c_str(), &startup, &process);
    if (!started) {
        started = CreateProcessW(nullptr, line.data(), nullptr, nullptr, FALSE, flags & ~CREATE_BREAKAWAY_FROM_JOB, nullptr,
                                 directory.c_str(), &startup, &process);
    }
    if (process.hProcess) {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    return started ? 0 : 1;
}
#endif

// Whether the background part is listening. Its channel is a named pipe on
// Windows and a socket file on Linux; either way it is a local socket.
static bool channelAnswers(const QString &channel)
{
    QLocalSocket socket;
    socket.connectToServer(channel);
    const bool up = socket.waitForConnected(300);
    socket.abort();
    return up;
}

// Starts the background part of this installation with the same state folder and
// channel the login entry uses, so one computer keeps one identity whichever of
// the two started it. The window does this itself because a packaged app must
// work when it is simply launched, not only after a sign-in.
static bool startBackground(const QString &channel)
{
    const QDir app_dir(QCoreApplication::applicationDirPath());
#ifdef Q_OS_WIN
    const QString service = app_dir.filePath("secpar-service.exe");
    const QString engine = app_dir.filePath("engine/sunshine.exe");
    const QString state = qEnvironmentVariable("LOCALAPPDATA") + "/Secpar4";
#else
    const QString service = app_dir.filePath("secpar-service");
    const QString engine = app_dir.filePath("engine/sunshine");
    const QString state = QDir::homePath() + "/.local/share/Secpar4";
#endif
    if (!QFile::exists(service) || !QFile::exists(engine))
        return false;
    QDir().mkpath(state);
    const QStringList arguments{"--state", QDir::toNativeSeparators(state), "--channel", channel,
                                "--engine", QDir::toNativeSeparators(engine), "--tunnel-port", "46900"};
    return QProcess::startDetached(service, arguments, app_dir.absolutePath());
}

int main(int argc, char *argv[])
{
#ifdef Q_OS_WIN
    // Used by the installer and the login entry: start the background part and
    // leave. Checked before any GUI application exists, because those entries can
    // run with no desktop to draw on, and a window that cannot be created must not
    // stop hosting from starting.
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--start-host") == 0) {
            QCoreApplication host(argc, argv);
            return startHost();
        }
    }
#endif
    // Qt Labs Platform (the tray icon and its menu) is implemented on Qt
    // Widgets; a plain QGuiApplication leaves it with no menu at all.
    QApplication app(argc, argv);
    app.setApplicationName("Secpar");
    app.setApplicationVersion(SECPAR_VERSION);
    QQuickStyle::setStyle("Basic");
#if !defined(Q_OS_WIN)
    // KDE keeps its interface font in kdeglobals. Without the KDE platform theme
    // Qt falls back to fontconfig's default (DejaVu Sans here), which looks
    // foreign on a Plasma desktop, so take the family the person actually chose.
    {
        const QString globals = QDir::homePath() + "/.config/kdeglobals";
        if (QFile::exists(globals)) {
            QSettings settings(globals, QSettings::IniFormat);
            const QStringList parts = settings.value("General/font").toString().split(',');
            if (!parts.isEmpty() && !parts.first().trimmed().isEmpty()) {
                QFont chosen = app.font();
                chosen.setFamily(parts.first().trimmed());
                bool ok = false;
                const int points = parts.size() > 1 ? parts.at(1).toInt(&ok) : 0;
                if (ok && points > 0) chosen.setPointSize(points);
                app.setFont(chosen);
            }
        }
    }
#endif

    const QStringList args = app.arguments();
    auto value = [&](const QString &name) {
        const int i = args.indexOf(name);
        return i >= 0 && i + 1 < args.size() ? args.at(i + 1) : QString();
    };
    const QString shot = value("--screenshot");
    const QStringList size = value("--size").split('x');

    // The layout sample never touches the account or the password store.
    const bool sample = args.contains("--sample");
    QString channel = value("--channel");
    if (channel.isEmpty()) {
#ifdef Q_OS_WIN
        channel = "secpar-service";
#else
        channel = QDir(QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation)).filePath("secpar-service.sock");
#endif
    }
    // A launched app must host as well as connect. If nothing answers on the
    // channel, start this installation's background part and give it a moment:
    // its first start creates the device identity, which can take a few seconds.
    if (!sample && !channelAnswers(channel)) {
        startBackground(channel);
        for (int waited = 0; waited < 50 && !channelAnswers(channel); ++waited)
            QThread::msleep(200);
    }
    QJsonObject configuration{{"channel", channel}, {"memory_store", sample || args.contains("--memory-store")}};
    // The stream component is an internal part of this app. The packaged app
    // places it in stream/; a development build may have it beside the exe.
#ifdef Q_OS_WIN
    const QString stream_name = "SecparStream.exe";
#else
    const QString stream_name = "secpar-stream";
#endif
    const QDir app_dir(QCoreApplication::applicationDirPath());
    QString component = app_dir.filePath("stream/" + stream_name);
    if (!QFile::exists(component))
        component = app_dir.filePath(stream_name);
    if (QFile::exists(component) && !sample) {
        configuration["stream_component"] = QDir::toNativeSeparators(component);
        configuration["data_folder"] = QDir::toNativeSeparators(QStandardPaths::writableLocation(QStandardPaths::AppLocalDataLocation));
    }
    const int test = args.indexOf("--test-account");
    if (test >= 0 && test + 2 < args.size())
        configuration["test_account"] = QJsonArray{args.at(test + 1), args.at(test + 2)};
    Backend backend(configuration, value("--computer-name"));
    // An installed update starts its own copy; this one has to let go of the
    // files first.
    QObject::connect(&backend, &Backend::restartForUpdate, &app, &QCoreApplication::quit);
    if (args.contains("--test-update-check")) {
        QTimer::singleShot(500, &backend, [&backend] { backend.checkForUpdates(); });
    }
    QFile accounts(value("--test-sign-in"));
    if (accounts.open(QIODevice::ReadOnly)) {
        const QJsonObject first = QJsonDocument::fromJson(accounts.readAll()).object()["accounts"].toArray().first().toObject();
        backend.signIn(first["email"].toString(), first["password"].toString());
    }

    if (args.contains("--test-connect")) {
        QTimer::singleShot(3000, &backend, [&backend] {
            for (const QVariant &entry : backend.property("devices").toList()) {
                const QVariantMap device = entry.toMap();
                if (device["online"].toBool() && !device["this"].toBool()) {
                    backend.connectTo(device["id"].toString());
                    break;
                }
            }
        });
    }

    QQmlApplicationEngine engine;
    QQmlContext *context = engine.rootContext();
    context->setContextProperty("backend", &backend);
    // The stills the background part sends, as QML images.
    engine.addImageProvider("preview", new PreviewProvider(&backend));
    context->setContextProperty("appVersion", QString(SECPAR_VERSION));
    context->setContextProperty("sampleData", args.contains("--sample"));
    context->setContextProperty("sampleOpen", args.contains("--sample") && args.contains("--sample-open"));
    context->setContextProperty("startTab", value("--tab").toInt());
    context->setContextProperty("startSection", value("--section").toInt());
    // Review only: show the update row's confirmation step in a capture.
    context->setContextProperty("updateConfirmHint", args.contains("--update-confirm"));
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app,
                     [] { QCoreApplication::exit(1); }, Qt::QueuedConnection);
    engine.loadFromModule("Secpar", "Main");
    if (engine.rootObjects().isEmpty())
        return 1;

    auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().first());
    if (window && size.size() == 2) {
        window->setWidth(size.at(0).toInt());
        window->setHeight(size.at(1).toInt());
    }
    if (window && !shot.isEmpty()) {
        const int delay = value("--screenshot-delay").toInt();
        QTimer::singleShot(delay > 0 ? delay : 1200, window, [window, shot] {
            const bool saved = window->grabWindow().save(shot);
            QCoreApplication::exit(saved ? 0 : 2);
        });
    }
    return app.exec();
}
