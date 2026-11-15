#include "backend.h"

#include <QJsonArray>
#include <QCoreApplication>
#include <QJsonDocument>
#include <QSysInfo>
#include <QThreadPool>
#include <QTimer>

extern "C" {
char *secpar_app_call(const char *request);
void secpar_app_free(char *answer);
}

static QJsonObject call(const QJsonObject &request)
{
    QByteArray text = QJsonDocument(request).toJson(QJsonDocument::Compact);
    char *answer = secpar_app_call(text.constData());
    // The request may hold a password. Do not leave it in freed memory.
    text.fill('\0');
    const QJsonObject parsed = answer ? QJsonDocument::fromJson(answer).object() : QJsonObject();
    secpar_app_free(answer);
    return parsed;
}

Backend::Backend(const QJsonObject &configuration, const QString &computerName, QObject *parent)
    : QObject(parent), m_computer(computerName.isEmpty() ? QSysInfo::machineHostName() : computerName)
{
    QJsonObject request = configuration;
    request["do"] = "configure";
    call(request);
    // The stream runs in its own window. Check now and then whether it is still
    // there, so closing that window returns the computer tile to Connect.
    m_watch = new QTimer(this);
    m_watch->setInterval(2000);
    connect(m_watch, &QTimer::timeout, this, &Backend::pollState);
    connect(QCoreApplication::instance(), &QCoreApplication::aboutToQuit, this, &Backend::shutdown);
    refresh();
}

void Backend::ask(const QJsonObject &request, std::function<void(const QJsonObject &)> then, bool busy)
{
    if (busy) {
        m_busy = true;
        emit changed();
    }
    QThreadPool::globalInstance()->start([this, request, then = std::move(then), busy] {
        const QJsonObject answer = call(request);
        QMetaObject::invokeMethod(this, [this, answer, then, busy] {
            if (busy)
                m_busy = false;
            then(answer);
            emit changed();
        }, Qt::QueuedConnection);
    });
}

QString Backend::explain(const QJsonObject &answer)
{
    const QString code = answer["error"].toString();
    // The account service words its own refusals for the person signing in.
    if (!answer["message"].toString().isEmpty())
        return answer["message"].toString();
    if (code == "unreachable")
        return "The account service did not answer. Check the internet connection and try again.";
    if (code == "store_unavailable")
        return "The system's password store could not be used. Unlock it and try again.";
    if (code == "background_not_running")
        return "This computer was not added to the account because Secpar's background part is not running.";
    if (code == "offline")
        return "That computer is off or not connected to the internet.";
    if (code == "not_reachable")
        return "That computer did not answer. Check that both computers are on the same network.";
    if (code == "stream_component_missing" || code == "stream_component_failed" || code == "data_folder_unusable")
        return "The stream could not be started on this computer (" + code + ").";
    if (code == "pairing_refused" || code == "pairing_failed")
        return "That computer did not accept this one (" + code + "). Try again.";
    if (code == "signed_out")
        return QString();
    return "That did not work (" + code + "). Try again.";
}

void Backend::takeState(const QJsonObject &state)
{
    m_loaded = true;
    if (!state["ok"].toBool()) {
        m_message = explain(state);
        return;
    }
    m_signedIn = state["signed_in"].toBool();
    m_handle = state["handle"].toString();
    if (!m_signedIn)
        m_devices.clear();
    m_streaming = m_signedIn && state["streaming"].toBool();
    if (!state["connected_to_host"].toBool()) {
        m_connectedId.clear();
        m_streaming = false;
        m_watch->stop();
    } else if (m_streaming && !m_watch->isActive()) {
        m_watch->start();
    }
}

void Backend::pollState()
{
    if (m_polling || m_connectedId.isEmpty())
        return;
    m_polling = true;
    ask({{"do", "state"}}, [this](const QJsonObject &state) {
        m_polling = false;
        takeState(state);
    }, false);
}

void Backend::shutdown()
{
    // The event loop is ending, so this runs here rather than on a worker thread.
    // A stream must not outlive the window that started it.
    m_watch->stop();
    if (m_connectedId.isEmpty())
        return;
    call({{"do", "disconnect"}});
    m_connectedId.clear();
    m_streaming = false;
}

void Backend::connectTo(const QString &id)
{
    m_message.clear();
    m_connectingId = id;
    ask({{"do", "connect"}, {"id", id}}, [this, id](const QJsonObject &answer) {
        m_connectingId.clear();
        if (answer["ok"].toBool()) {
            m_connectedId = id;
            m_streaming = answer["streaming"].toBool();
            if (m_streaming)
                m_watch->start();
        } else {
            m_message = explain(answer);
        }
    });
}

void Backend::disconnect()
{
    m_watch->stop();
    m_streaming = false;
    ask({{"do", "disconnect"}}, [this](const QJsonObject &) { m_connectedId.clear(); });
}

void Backend::loadDevices()
{
    ask({{"do", "devices"}}, [this](const QJsonObject &answer) {
        if (answer["ok"].toBool()) {
            m_devices = answer["devices"].toArray().toVariantList();
        } else if (answer["error"].toString() == "signed_out") {
            m_signedIn = false;
            m_devices.clear();
        } else {
            m_message = explain(answer);
        }
    });
}

void Backend::refresh()
{
    ask({{"do", "state"}}, [this](const QJsonObject &state) {
        takeState(state);
        if (m_signedIn)
            loadDevices();
    });
}

void Backend::signIn(const QString &email, const QString &password)
{
    m_message.clear();
    const QJsonObject request{{"do", "sign_in"}, {"email", email.trimmed()}, {"password", password},
                              {"computer", m_computer}};
    ask(request, [this](const QJsonObject &state) {
        takeState(state);
        if (!m_signedIn)
            return;
        if (!state["enrolled"].toBool())
            m_message = explain({{"error", state["enrol_error"]}});
        loadDevices();
    });
}

void Backend::signOut()
{
    m_message.clear();
    ask({{"do", "sign_out"}}, [this](const QJsonObject &answer) {
        if (answer["ok"].toBool() || answer["error"].toString() == "signed_out") {
            m_watch->stop();
            m_streaming = false;
            m_signedIn = false;
            m_handle.clear();
            m_devices.clear();
        } else {
            m_message = explain(answer);
        }
    });
}

void Backend::removeDevice(const QString &id)
{
    ask({{"do", "remove_device"}, {"id", id}}, [this](const QJsonObject &answer) {
        if (!answer["ok"].toBool())
            m_message = explain(answer);
        loadDevices();
    });
}
