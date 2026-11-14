// Secpar desktop app. One window that connects to computers and controls hosting.
//
// Development options:
//   --sample                 fill the pages with fictional layout data
//   --tab <0|1|2>            open on Computers, Friends or Settings
//   --size <W>x<H>           window size in logical pixels
//   --screenshot <file.png>  save the window to a file and quit
#include <QGuiApplication>
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

    QQmlApplicationEngine engine;
    QQmlContext *context = engine.rootContext();
    context->setContextProperty("appVersion", QString(SECPAR_VERSION));
    context->setContextProperty("sampleData", args.contains("--sample"));
    context->setContextProperty("startTab", value("--tab").toInt());
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
        QTimer::singleShot(1200, window, [window, shot] {
            const bool saved = window->grabWindow().save(shot);
            QCoreApplication::exit(saved ? 0 : 2);
        });
    }
    return app.exec();
}
