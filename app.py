import sys
from PyQt5.QtWidgets import QApplication, QMainWindow, QVBoxLayout, QWidget, QAction, QMenuBar
from PyQt5.QtWebEngineWidgets import QWebEngineView
from PyQt5.QtCore import QUrl
from threading import Thread
import requests
from main import app
import os

class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Flask App")
        self.setGeometry(100, 100, 1200, 800)

        self.browser = QWebEngineView()
        self.browser.setUrl(QUrl("http://localhost:5000"))

        self.setCentralWidget(self.browser)

        # Create a menu bar
        menubar = self.menuBar()
        file_menu = menubar.addMenu('File')

        # Add exit action
        exit_action = QAction('Exit', self)
        exit_action.triggered.connect(self.close)
        file_menu.addAction(exit_action)

    def closeEvent(self, event):
        print("Closing application...")
        QApplication.exit()
        os._exit(0)

def run_flask():
    app.run(port=5000)

if __name__ == "__main__":
    flask_thread = Thread(target=run_flask)
    flask_thread.daemon = True
    flask_thread.start()

    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()

    app.exec_()
    flask_thread.join()
    print("Flask server stopped.")
    sys.exit(0)