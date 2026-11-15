// What the pages read and ask for. The work itself is in the Secpar app library
// (crates/secpar-app): one JSON request in, one JSON answer out, each call
// blocking, so every call here runs on a worker thread.
#pragma once

#include <QJsonObject>
#include <QObject>
#include <QVariantList>
#include <QVariantMap>
#include <functional>

class QTimer;

class Backend : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool loaded MEMBER m_loaded NOTIFY changed)
    Q_PROPERTY(bool signedIn MEMBER m_signedIn NOTIFY changed)
    Q_PROPERTY(QString handle MEMBER m_handle NOTIFY changed)
    Q_PROPERTY(bool busy MEMBER m_busy NOTIFY changed)
    Q_PROPERTY(QString message MEMBER m_message NOTIFY changed)
    Q_PROPERTY(QVariantList devices MEMBER m_devices NOTIFY changed)
    // The computer this one has a tunnel to, and the one it is trying to reach.
    Q_PROPERTY(QString connectedId MEMBER m_connectedId NOTIFY changed)
    Q_PROPERTY(QString connectingId MEMBER m_connectingId NOTIFY changed)
    // True while the stream window for the connected computer is running.
    Q_PROPERTY(bool streaming MEMBER m_streaming NOTIFY changed)
    // Settings → Connecting, and the hosting state Settings → Hosting shows.
    Q_PROPERTY(QVariantMap preferences MEMBER m_preferences NOTIFY changed)
    Q_PROPERTY(bool hosting MEMBER m_hosting NOTIFY changed)
    Q_PROPERTY(int allowedDevices MEMBER m_allowedDevices NOTIFY changed)
    Q_PROPERTY(QString accessList MEMBER m_accessList NOTIFY changed)

public:
    explicit Backend(const QJsonObject &configuration, const QString &computerName, QObject *parent = nullptr);

    Q_INVOKABLE void refresh();
    Q_INVOKABLE void signIn(const QString &email, const QString &password);
    Q_INVOKABLE void signOut();
    Q_INVOKABLE void removeDevice(const QString &id);
    Q_INVOKABLE void connectTo(const QString &id);
    Q_INVOKABLE void disconnect();
    Q_INVOKABLE void setPreference(const QString &key, const QVariant &value);

signals:
    void changed();

private:
    void ask(const QJsonObject &request, std::function<void(const QJsonObject &)> then, bool busy = true);
    void takeState(const QJsonObject &state);
    void loadDevices();
    void pollState();
    void shutdown();
    static QString explain(const QJsonObject &answer);

    bool m_loaded = false;
    bool m_signedIn = false;
    bool m_busy = false;
    bool m_polling = false;
    QString m_handle;
    QString m_computer;
    QString m_message;
    QString m_connectedId;
    QString m_connectingId;
    bool m_streaming = false;
    QVariantMap m_preferences;
    bool m_hosting = false;
    int m_allowedDevices = 0;
    QString m_accessList;
    QVariantList m_devices;
    QTimer *m_watch = nullptr;
};
