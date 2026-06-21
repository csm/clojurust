(ns clojure.string-test.replace
  (:require [clojure.string :as str]
            [clojure.test :as t :refer [deftest testing is]]))

(deftest test-replace-string-match
  (is (= "bbb" (str/replace "aaa" "a" "b")))
  (is (= "bbc" (str/replace "aac" "a" "b")))
  (is (= "hello" (str/replace "hello" "x" "y")))
  (is (= "" (str/replace "aaa" "a" ""))))

(deftest test-replace-pattern-match
  (is (= "host" (str/replace "--host" #"^--" "")))
  (is (= "bbb" (str/replace "aaa" #"a" "b")))
  (is (= "hello world" (str/replace "hello   world" #"\s+" " ")))
  (is (= "" (str/replace "123" #"\d+" "")))
  (is (= "123" (str/replace "123" #"[a-z]+" ""))))
