// Secpar desktop app. One window that connects to computers and controls hosting.
//
// Development options:
//   --sample                 fill the pages with fictional layout data
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
#include "backend.h"

#include <QDir>
#include <QFile>
#include <QGuiApplication>
#include <QJsonArray>
#include <QJsonDocument>
#include <QStandardPaths>
#include <QImage>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QTimer>

int main(int argc, char *argv[])
{
    QGuiApplication app(argc, argv);
    app.setApplicationName("Secpar");
    app.setApplicationVersion(SECPAR_VERSION);
    QQuickStyle::setStyle("Basic");

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
    QJsonObject configuration{{"channel", channel}, {"memory_store", sample || args.contains("--memory-store")}};
    const int test = args.indexOf("--test-account");
    if (test >= 0 && test + 2 < args.size())
        configuration["test_account"] = QJsonArray{args.at(test + 1), args.at(test + 2)};
    Backend backend(configuration, value("--computer-name"));
    QFile accounts(value("--test-sign-in"));
    if (accounts.open(QIODevice::ReadOnly)) {
        const QJsonObject first = QJsonDocument::fromJson(accounts.readAll()).object()["accounts"].toArray().first().toObject();
        backend.signIn(first["email"].toString(), first["password"].toString());
    }

    QQmlApplicationEngine engine;
    QQmlContext *context = engine.rootContext();
    context->setContextProperty("backend", &backend);
    context->setContextProperty("appVersion", QString(SECPAR_VERSION));
    context->setContextProperty("sampleData", args.contains("--sample"));
    context->setContextProperty("startTab", value("--tab").toInt());
    context->setContextProperty("startSection", value("--section").toInt());
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
